use std::{fmt::Debug, future::Future, time::Duration};

use crate::{
    cluster::cluster_uri, impl_into_string, stream_err, BaseOptionKey, KafkaConnectOptions,
    KafkaErr, KafkaResult,
};
use rdkafka::{
    config::ClientConfig,
    producer::{DeliveryFuture, FutureRecord as RawPayload, Producer as ProducerTrait},
};
pub use rdkafka::{consumer::ConsumerGroupMetadata, TopicPartitionList};
use sea_streamer_runtime::spawn_blocking;
use sea_streamer_types::{
    export::futures::FutureExt, runtime_error, Buffer, MessageHeader, Producer, ProducerOptions,
    ShardId, StreamErr, StreamKey, StreamResult, StreamerUri, Timestamp,
};

#[derive(Clone)]
pub struct KafkaProducer {
    stream: Option<StreamKey>,
    inner: Option<RawProducer>,
}

impl Debug for KafkaProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KafkaProducer")
            .field("stream", &self.stream)
            .finish()
    }
}

/// rdkafka's FutureProducer type
pub type RawProducer = rdkafka::producer::FutureProducer<
    rdkafka::client::DefaultClientContext,
    crate::KafkaAsyncRuntime,
>;

#[derive(Debug, Default, Clone)]
pub struct KafkaProducerOptions {
    compression_type: Option<CompressionType>,
    custom_options: Vec<(String, String)>,
}

pub struct SendFuture {
    stream_key: StreamKey,
    fut: DeliveryFuture,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum KafkaProducerOptionKey {
    CompressionType,
}

type OptionKey = KafkaProducerOptionKey;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CompressionType {
    None,
    Gzip,
    Snappy,
    Lz4,
    Zstd,
}

impl Default for CompressionType {
    fn default() -> Self {
        Self::None
    }
}

impl Debug for SendFuture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendFuture")
            .field("stream_key", &self.stream_key)
            .finish()
    }
}

impl Producer for KafkaProducer {
    type Error = KafkaErr;
    type SendFuture = SendFuture;

    fn send_to<S: Buffer>(&self, stream: &StreamKey, payload: S) -> KafkaResult<Self::SendFuture> {
        let fut = self
            .get()
            .send_result(RawPayload::<str, [u8]>::to(stream.name()).payload(payload.as_bytes()))
            .map_err(|(err, _raw)| stream_err(err))?;

        let stream_key = stream.to_owned();
        Ok(SendFuture { stream_key, fut })
    }

    fn anchor(&mut self, stream: StreamKey) -> KafkaResult<()> {
        if self.stream.is_none() {
            self.stream = Some(stream);
            Ok(())
        } else {
            Err(StreamErr::AlreadyAnchored)
        }
    }

    fn anchored(&self) -> KafkaResult<&StreamKey> {
        if let Some(stream) = &self.stream {
            Ok(stream)
        } else {
            Err(StreamErr::NotAnchored)
        }
    }
}

impl KafkaProducer {
    /// Get the underlying FutureProducer
    #[inline]
    fn get(&self) -> &RawProducer {
        self.inner
            .as_ref()
            .expect("Producer is still inside a transaction, please await the future")
    }

    /// Returns the number of messages that are either waiting to be sent or
    /// are sent but are waiting to be acknowledged.
    pub fn in_flight_count(&self) -> i32 {
        self.get().in_flight_count()
    }

    #[inline]
    async fn transaction<F: FnOnce(&RawProducer) -> Result<(), KafkaErr> + Send + 'static>(
        &mut self,
        func: F,
    ) -> KafkaResult<()> {
        self.get();
        let client = self.inner.take().unwrap();
        match spawn_blocking(move || {
            let s = client;
            match func(&s) {
                Ok(()) => Ok(s),
                Err(e) => Err((s, e)),
            }
        })
        .await
        .map_err(runtime_error)?
        {
            Ok(inner) => {
                self.inner = Some(inner);
                Ok(())
            }
            Err((inner, err)) => {
                self.inner = Some(inner);
                Err(stream_err(err))
            }
        }
    }

    /// See https://docs.rs/rdkafka/latest/rdkafka/producer/trait.Producer.html#tymethod.init_transactions
    ///
    /// # Warning
    ///
    /// This async method is not cancel safe. You must await this future,
    /// and this Producer will be unusable for any operations until it finishes.
    pub async fn init_transactions(&mut self, timeout: Duration) -> KafkaResult<()> {
        self.transaction(move |s| s.init_transactions(timeout))
            .await
    }

    /// See https://docs.rs/rdkafka/latest/rdkafka/producer/trait.Producer.html#tymethod.begin_transaction
    ///
    /// # Warning
    ///
    /// This async method is not cancel safe. You must await this future,
    /// and this Producer will be unusable for any operations until it finishes.
    pub async fn begin_transaction(&mut self) -> KafkaResult<()> {
        self.transaction(|s| s.begin_transaction()).await
    }

    /// See https://docs.rs/rdkafka/latest/rdkafka/producer/trait.Producer.html#tymethod.commit_transaction
    ///
    /// # Warning
    ///
    /// This async method is not cancel safe. You must await this future,
    /// and this Producer will be unusable for any operations until it finishes.
    pub async fn commit_transaction(&mut self, timeout: Duration) -> KafkaResult<()> {
        self.transaction(move |s| s.commit_transaction(timeout))
            .await
    }

    /// See https://docs.rs/rdkafka/latest/rdkafka/producer/trait.Producer.html#tymethod.abort_transaction
    ///
    /// # Warning
    ///
    /// This async method is not cancel safe. You must await this future,
    /// and this Producer will be unusable for any operations until it finishes.
    pub async fn abort_transaction(&mut self, timeout: Duration) -> KafkaResult<()> {
        self.transaction(move |s| s.abort_transaction(timeout))
            .await
    }

    /// https://docs.rs/rdkafka/latest/rdkafka/producer/trait.Producer.html#tymethod.send_offsets_to_transaction
    ///
    /// # Warning
    ///
    /// This async method is not cancel safe. You must await this future,
    /// and this Producer will be unusable for any operations until it finishes.
    pub async fn send_offsets_to_transaction(
        &mut self,
        offsets: TopicPartitionList,
        cgm: ConsumerGroupMetadata,
        timeout: Duration,
    ) -> KafkaResult<()> {
        self.transaction(move |s| s.send_offsets_to_transaction(&offsets, &cgm, timeout))
            .await
    }

    /// Flush pending messages. This method blocks.
    pub(crate) fn flush_sync(&self, timeout: Duration) -> KafkaResult<()> {
        self.get().flush(timeout).map_err(stream_err)
    }

    /// Flush pending messages.
    pub async fn flush(self, timeout: Duration) -> KafkaResult<()> {
        spawn_blocking(move || self.flush_sync(timeout))
            .await
            .map_err(runtime_error)?
    }
}

impl ProducerOptions for KafkaProducerOptions {}

impl KafkaProducerOptions {
    /// Set the compression method for this producer
    pub fn set_compression_type(&mut self, v: CompressionType) -> &mut Self {
        self.compression_type = Some(v);
        self
    }
    pub fn compression_type(&self) -> Option<&CompressionType> {
        self.compression_type.as_ref()
    }

    /// Add a custom option. If you have an option you frequently use,
    /// please consider open a PR and add it to above.
    pub fn add_custom_option<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.custom_options.push((key.into(), value.into()));
        self
    }
    pub fn custom_options(&self) -> impl Iterator<Item = (&str, &str)> {
        self.custom_options
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    fn make_client_config(&self, client_config: &mut ClientConfig) {
        if let Some(v) = self.compression_type {
            client_config.set(OptionKey::CompressionType, v);
        }
        for (key, value) in self.custom_options() {
            client_config.set(key, value);
        }
    }
}

impl OptionKey {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CompressionType => "compression.type",
        }
    }
}

impl CompressionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Gzip => "gzip",
            Self::Snappy => "snappy",
            Self::Lz4 => "lz4",
            Self::Zstd => "zstd",
        }
    }
}

impl_into_string!(OptionKey);
impl_into_string!(CompressionType);

impl Future for SendFuture {
    type Output = StreamResult<MessageHeader, KafkaErr>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let stream_key = self.stream_key.to_owned();
        match self.fut.poll_unpin(cx) {
            std::task::Poll::Ready(res) => std::task::Poll::Ready(match res {
                Ok(res) => match res {
                    Ok((part, offset)) => Ok(MessageHeader::new(
                        stream_key,
                        ShardId::new(part as u64),
                        offset as u64,
                        Timestamp::now_utc(),
                    )),
                    Err((err, _)) => Err(stream_err(err)),
                },
                Err(_) => Err(stream_err(KafkaErr::Canceled)),
            }),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

pub(crate) fn create_producer(
    streamer: &StreamerUri,
    base_options: &KafkaConnectOptions,
    options: &KafkaProducerOptions,
) -> Result<KafkaProducer, KafkaErr> {
    let mut client_config = ClientConfig::new();
    client_config.set(BaseOptionKey::BootstrapServers, cluster_uri(streamer)?);
    base_options.make_client_config(&mut client_config);
    options.make_client_config(&mut client_config);

    let producer = client_config.create()?;

    Ok(KafkaProducer {
        stream: None,
        inner: Some(producer),
    })
}
