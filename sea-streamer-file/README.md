### `sea-streamer-file`: File Backend

This is very similar to `sea-streamer-stdio`, but the difference is SeaStreamerStdio works in real-time,
while `sea-streamer-file` works in real-time and replay. That means, SeaStreamerFile has the ability to
traverse a `.ss` (sea-stream) file and seek/rewind to a particular timestamp/offset.

In addition, Stdio can only work with UTF-8 text data, while File is able to work with binary data.
In Stdio, there is only one Streamer per process. In File, there can be multiple independent Streamers
in the same process. Afterall, a Streamer is just a file.

The basic idea of SeaStreamerFile is like a `tail -f` with one message per line, with a custom message frame
carrying binary payloads. The interesting part is, in SeaStreamer, we do not use delimiters to separate messages.
This removes the overhead of encoding/decoding message payloads. But it added some complexity to the file format.

The SeaStreamerFile format is designed for efficient fast-forward and seeking. This is enabled by placing an array
of Beacons at fixed interval in the file. A Beacon contains a summary of the streams, so it acts like an inplace
index. It also allows readers to align with the message boundaries. To learn more about the file format, read
[`src/format.rs`](https://github.com/SeaQL/sea-streamer/blob/main/sea-streamer-file/src/format.rs).

On top of that, are the high-level SeaStreamer multi-producer, multi-consumer stream semantics, resembling
the behaviour of other SeaStreamer backends. In particular, the load-balancing behaviour is same as Stdio,
i.e. round-robin.

### Decoder

We provide a small utility to decode `.ss` files:

```sh
alias decoder='cargo run --package sea-streamer-file --features=executables --bin decoder'
decoder -- --file <file> --format <format>
 # or if you wish to install it
cargo install sea-streamer-file --features=executables --bin decoder
```

Example `log` format:

```log
 # header
[2023-06-05T13:55:53.001 | hello | 1 | 0] message-1
 # beacon
```

Example `ndjson` format:

```json
/* header */
{"header":{"stream_key":"hello","shard_id":0,"sequence":1,"timestamp":"2023-06-05T13:55:53.001"},"payload":"message-1"}
/* beacon */
```

There is also a Typescript implementation under [`sea-streamer-file-reader`](https://github.com/SeaQL/sea-streamer/tree/main/sea-streamer-file/sea-streamer-file-reader).

### TODO

1. Resumable: currently unimplemented. A potential implementation might be to commit into a local SQLite database.
2. Sharding: currently it only streams to Shard ZERO.
3. Verify: a utility program to verify and repair SeaStreamer binary file.
