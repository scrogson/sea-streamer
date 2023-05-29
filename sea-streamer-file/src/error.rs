use crate::format::{FormatErr, HeaderErr};
use sea_streamer_types::{StreamErr, StreamResult};
use std::str::Utf8Error;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FileErr {
    #[error("Utf8Error: {0}")]
    Utf8Error(#[source] Utf8Error),
    #[error("IO Error: {0}")]
    IoError(#[source] std::io::Error),
    #[error("Duplicate IoError")]
    DuplicateIoError,
    #[error("Watch Error: {0}")]
    WatchError(String),
    #[error("HeaderErr: {0}")]
    HeaderErr(#[source] HeaderErr),
    #[error("FormatErr: {0}")]
    FormatErr(#[source] FormatErr),
    #[error("File Removed")]
    FileRemoved,
    #[error("File Limit Exceeded")]
    FileLimitExceeded,
    #[error("Task Dead ({0})")]
    TaskDead(&'static str),
    #[error("Not Enough Bytes: the file might be truncated.")]
    NotEnoughBytes,
    #[error("Flume RecvError: {0}")]
    RecvError(flume::RecvError),
}

pub type FileResult<T> = StreamResult<T, FileErr>;

impl FileErr {
    /// Take ownership of this Err, leaving a clone in place.
    pub fn take(&mut self) -> Self {
        let mut copy = match self {
            FileErr::Utf8Error(e) => FileErr::Utf8Error(e.clone()),
            FileErr::IoError(_) => FileErr::DuplicateIoError,
            FileErr::DuplicateIoError => FileErr::DuplicateIoError,
            FileErr::WatchError(e) => FileErr::WatchError(e.clone()),
            FileErr::HeaderErr(e) => FileErr::HeaderErr(e.clone()),
            FileErr::FormatErr(e) => FileErr::FormatErr(e.clone()),
            FileErr::FileRemoved => FileErr::FileRemoved,
            FileErr::FileLimitExceeded => FileErr::FileLimitExceeded,
            FileErr::TaskDead(e) => FileErr::TaskDead(e.clone()),
            FileErr::NotEnoughBytes => FileErr::NotEnoughBytes,
            FileErr::RecvError(e) => FileErr::RecvError(e.clone()),
        };
        std::mem::swap(self, &mut copy);
        copy
    }
}

impl From<FileErr> for StreamErr<FileErr> {
    fn from(err: FileErr) -> Self {
        StreamErr::Backend(err)
    }
}
