use flume::{bounded, unbounded, Receiver, Sender, TryRecvError};

use crate::{
    watcher::{new_watcher, FileEvent, Watcher},
    AsyncFile, Bytes, FileErr,
};
use sea_streamer_runtime::spawn_task;

pub trait ByteSink {
    /// This should never block.
    fn write(&mut self, bytes: Bytes) -> Result<(), FileErr>;
}

/// Buffered file writer.
///
/// If the file is removed from the file system, the stream ends.
pub struct FileSink {
    watcher: Option<Watcher>,
    sender: Sender<Request>,
    update: Receiver<Update>,
}

#[derive(Debug)]
enum Request {
    Bytes(Bytes),
    Flush(u32),
    SyncAll,
}

#[derive(Debug)]
enum Update {
    FileErr(FileErr),
    Receipt(u32),
}

impl FileSink {
    pub fn new(mut file: AsyncFile, mut quota: u64) -> Result<Self, FileErr> {
        let (sender, pending) = unbounded();
        let (notify, update) = bounded(0);
        let (watch, event) = unbounded();
        let watcher = new_watcher(file.id(), watch)?;
        quota -= file.size();

        let _handle = spawn_task(async move {
            'outer: while let Ok(request) = pending.recv_async().await {
                match request {
                    Request::Bytes(mut bytes) => {
                        let mut len = bytes.len() as u64;
                        if quota < len {
                            bytes = bytes.pop(quota as usize);
                            len = quota;
                        }

                        if let Err(err) = file.write_all(bytes).await {
                            std::mem::drop(pending); // trigger error
                            send_error(&notify, err).await;
                            break;
                        }

                        quota -= len;
                        if quota == 0 {
                            std::mem::drop(pending); // trigger error
                            send_error(&notify, FileErr::FileLimitExceeded).await;
                            break;
                        }
                    }
                    Request::Flush(marker) => {
                        if let Err(err) = file.flush().await {
                            std::mem::drop(pending); // trigger error
                            send_error(&notify, err).await;
                            break;
                        }
                        if notify.send_async(Update::Receipt(marker)).await.is_err() {
                            break;
                        }
                    }
                    Request::SyncAll => {
                        if let Err(err) = file.sync_all().await {
                            std::mem::drop(pending); // trigger error
                            send_error(&notify, err).await;
                            break;
                        }
                        if notify.send_async(Update::Receipt(u32::MAX)).await.is_err() {
                            break;
                        }
                    }
                }

                loop {
                    match event.try_recv() {
                        Ok(FileEvent::Modify) => {}
                        Ok(FileEvent::Remove) => {
                            std::mem::drop(pending); // trigger error
                            send_error(&notify, FileErr::FileRemoved).await;
                            break 'outer;
                        }
                        Ok(FileEvent::Error(e)) => {
                            std::mem::drop(pending); // trigger error
                            send_error(&notify, FileErr::WatchError(e)).await;
                            break 'outer;
                        }
                        Err(TryRecvError::Disconnected) => {
                            break 'outer;
                        }
                        Ok(FileEvent::Rewatch) => {
                            log::warn!("Why are we receiving this?");
                            break 'outer;
                        }
                        Err(TryRecvError::Empty) => break,
                    }
                }
            }
            log::debug!("FileSink task finish ({})", file.id().path());
        });

        async fn send_error(notify: &Sender<Update>, e: FileErr) {
            if let Err(e) = notify.send_async(Update::FileErr(e)).await {
                log::error!("{:?}", e.into_inner());
            }
        }

        Ok(Self {
            watcher: Some(watcher),
            sender,
            update,
        })
    }

    fn return_err(&mut self) -> Result<(), FileErr> {
        if self.watcher.is_some() {
            // kill the watcher so we don't leak
            self.watcher.take();
        }

        Err(loop {
            match self.update.try_recv() {
                Ok(Update::FileErr(err)) => break err,
                Ok(_) => (),
                Err(err) => {
                    panic!("The task should always wait until the error has been sent: {err}")
                }
            }
        })
    }

    pub async fn flush(&mut self, marker: u32) -> Result<(), FileErr> {
        if self.sender.send(Request::Flush(marker)).is_err() {
            self.return_err()
        } else {
            match self.update.recv_async().await {
                Ok(Update::Receipt(receipt)) => {
                    assert_eq!(receipt, marker);
                    Ok(())
                }
                Ok(Update::FileErr(err)) => Err(err),
                Err(_) => Err(FileErr::TaskDead("sink")),
            }
        }
    }

    pub async fn sync_all(&mut self) -> Result<(), FileErr> {
        if self.sender.send(Request::SyncAll).is_err() {
            self.return_err()
        } else {
            loop {
                match self.update.recv_async().await {
                    Ok(Update::Receipt(u32::MAX)) => return Ok(()),
                    Ok(Update::Receipt(_)) => (),
                    Ok(Update::FileErr(err)) => return Err(err),
                    Err(_) => return Err(FileErr::TaskDead("sink")),
                }
            }
        }
    }
}

impl ByteSink for FileSink {
    /// This method never blocks
    fn write(&mut self, bytes: Bytes) -> Result<(), FileErr> {
        if self.sender.send(Request::Bytes(bytes)).is_err() {
            self.return_err()
        } else {
            Ok(())
        }
    }
}
