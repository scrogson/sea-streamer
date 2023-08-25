use anyhow::{anyhow, Result};
use sea_streamer_file::{FileId, FileStreamer};
use sea_streamer_types::{Producer, StreamKey, Streamer};
use std::time::{Duration, Instant};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
struct Args {
    #[structopt(long, help = "Stream to this file")]
    file: FileId,
    #[structopt(long, parse(try_from_str = parse_duration), help = "Period of the clock. e.g. 1s, 100ms")]
    interval: Duration,
}

fn parse_duration(src: &str) -> Result<Duration> {
    if let Some(s) = src.strip_suffix("ms") {
        Ok(Duration::from_millis(s.parse()?))
    } else if let Some(s) = src.strip_suffix('s') {
        Ok(Duration::from_secs(s.parse()?))
    } else if let Some(s) = src.strip_suffix('m') {
        Ok(Duration::from_secs(s.parse::<u64>()? * 60))
    } else {
        Err(anyhow!("Failed to parse {} as Duration", src))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let Args { file, interval } = Args::from_args();

    let stream_key = StreamKey::new("clock")?;
    let streamer = FileStreamer::connect(file.to_streamer_uri()?, Default::default()).await?;
    let producer = streamer
        .create_producer(stream_key, Default::default())
        .await?;

    for i in 0..u64::MAX {
        let next = Instant::now() + interval;
        producer.send(format!("tick-{i}"))?;
        let now = Instant::now();
        if let Some(dur) = next.checked_duration_since(now) {
            tokio::time::sleep(dur).await;
        } else {
            tokio::time::sleep(Duration::from_nanos(1)).await;
        }
    }

    producer.end().await?;

    Ok(())
}
