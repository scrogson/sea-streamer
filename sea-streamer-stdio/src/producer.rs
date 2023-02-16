use flume::{bounded, r#async::RecvFut, unbounded, Sender};
use std::{collections::HashMap, fmt::Debug, future::Future, sync::Mutex};

use sea_streamer_types::{
    export::futures::FutureExt, Message, MessageHeader, Producer as ProducerTrait, Receipt,
    Sendable, SequenceNo, ShardId, SharedMessage, StreamErr, StreamKey, StreamResult, Timestamp,
};

use crate::{parser::TIME_FORMAT, StdioErr, StdioResult};

lazy_static::lazy_static! {
    static ref PRODUCERS: Mutex<Producers> = Default::default();
    static ref THREAD: Mutex<Option<Sender<Signal>>> = Mutex::new(None);
}

#[derive(Debug, Default)]
struct Producers {
    sequences: HashMap<StreamKey, SequenceNo>,
}

enum Signal {
    SendRequest {
        message: SharedMessage,
        receipt: Sender<Receipt>,
    },
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct StdioProducer {
    stream: Option<StreamKey>,
    request: Sender<Signal>,
}

pub struct SendFuture {
    fut: RecvFut<'static, Receipt>,
}

const ZERO: u64 = 0;

pub(crate) fn init() {
    let mut thread = THREAD.lock().expect("Failed to lock stdout thread");
    if thread.is_none() {
        let (sender, receiver) = unbounded();
        std::thread::spawn(move || {
            log::info!("[{pid}] stdout thread spawned", pid = std::process::id());
            // this thread locks the mutex forever
            let mut producers = PRODUCERS
                .try_lock()
                .expect("Should have no other thread trying to access Producers");
            while let Ok(signal) = receiver.recv() {
                match signal {
                    Signal::SendRequest {
                        mut message,
                        receipt,
                    } => {
                        // we can time the difference from send() until now()
                        message.touch(); // set timestamp to now

                        // I believe println is atomic now, so we don't have to lock stdout
                        // fn main() {
                        //     std::thread::scope(|s| {
                        //         for num in 0..100 {
                        //             s.spawn(move || {
                        //                 println!("Hello from thread number {}", num);
                        //             });
                        //         }
                        //     });
                        // }
                        let stream_key = message.stream_key();
                        println!(
                            "[{timestamp} | {stream} | {seq}] {payload}",
                            timestamp = message
                                .timestamp()
                                .format(TIME_FORMAT)
                                .expect("Timestamp format error"),
                            stream = stream_key,
                            seq = producers.append(&stream_key),
                            payload = message
                                .message()
                                .as_str()
                                .expect("Should have already checked is valid string"),
                        );
                        let meta = message.take_meta();
                        // we don't care if the receipt can be delivered
                        receipt.send(meta).ok();
                    }
                    Signal::Shutdown => break,
                }
            }
            log::info!("[{pid}] stdout thread exit", pid = std::process::id());
            {
                let mut thread = THREAD.lock().expect("Failed to lock stdout thread");
                thread.take(); // set to none
            }
        });
        thread.replace(sender);
    }
}

pub(crate) fn shutdown() {
    let thread = THREAD.lock().expect("Failed to lock stdout thread");
    if let Some(sender) = thread.as_ref() {
        sender
            .send(Signal::Shutdown)
            .expect("stdout thread might have been shutdown already");
    }
}

pub(crate) fn shutdown_already() -> bool {
    let thread = THREAD.lock().expect("Failed to lock stdout thread");
    thread.is_none()
}

impl Producers {
    // returns current Seq No
    fn append(&mut self, stream: &StreamKey) -> SequenceNo {
        if let Some(val) = self.sequences.get_mut(stream) {
            let seq = *val;
            *val += 1;
            seq
        } else {
            self.sequences.insert(stream.to_owned(), 1);
            0
        }
    }
}

impl Future for SendFuture {
    type Output = StreamResult<MessageHeader, StdioErr>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match self.fut.poll_unpin(cx) {
            std::task::Poll::Ready(res) => std::task::Poll::Ready(match res {
                Ok(res) => Ok(res),
                Err(err) => Err(StreamErr::Backend(StdioErr::RecvError(err))),
            }),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

impl Debug for SendFuture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendFuture").finish()
    }
}

impl ProducerTrait for StdioProducer {
    type Error = StdioErr;
    type SendFuture = SendFuture;

    fn send_to<S: Sendable>(
        &self,
        stream: &StreamKey,
        payload: S,
    ) -> StdioResult<Self::SendFuture> {
        let payload = payload.as_str().map_err(StreamErr::Utf8Error)?.to_owned();
        // basically using this as oneshot
        let (sender, receiver) = bounded(1);
        let size = payload.len();
        self.request
            .send(Signal::SendRequest {
                message: SharedMessage::new(
                    MessageHeader::new(
                        stream.to_owned(),
                        ShardId::new(ZERO),
                        ZERO,
                        Timestamp::now_utc(),
                    ),
                    payload.into_bytes(),
                    0,
                    size,
                ),
                receipt: sender,
            })
            .map_err(|_| StreamErr::Backend(StdioErr::Disconnected))?;
        Ok(SendFuture {
            fut: receiver.into_recv_async(),
        })
    }

    fn anchor(&mut self, stream: StreamKey) -> StdioResult<()> {
        if self.stream.is_none() {
            self.stream = Some(stream);
            Ok(())
        } else {
            Err(StreamErr::AlreadyAnchored)
        }
    }

    fn anchored(&self) -> StdioResult<&StreamKey> {
        if let Some(stream) = &self.stream {
            Ok(stream)
        } else {
            Err(StreamErr::NotAnchored)
        }
    }
}

impl StdioProducer {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        init();
        let request = {
            let thread = THREAD.lock().expect("Failed to lock stdout thread");
            thread.as_ref().expect("Should have initialized").to_owned()
        };
        Self {
            stream: None,
            request,
        }
    }
}
