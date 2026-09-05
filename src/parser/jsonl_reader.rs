//! Bounded, read-only JSONL tail. Newline is the record commit marker.
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};

pub enum Record<'a> {
    Line(&'a [u8]),
    Oversized,
}

pub struct BoundedReader {
    path: PathBuf,
    file: File,
    pub byte_offset: u64,
    pending: Vec<u8>,
    dropping: bool,
    max_record_bytes: usize,
    identity: FileIdentity,
    modified: Option<SystemTime>,
    prefix: Vec<u8>,
}

#[derive(PartialEq, Eq)]
struct FileIdentity {
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(not(unix))]
    created: Option<SystemTime>,
}
impl FileIdentity {
    fn of(m: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                dev: m.dev(),
                ino: m.ino(),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                created: m.created().ok(),
            }
        }
    }
}

impl BoundedReader {
    pub fn open(path: &Path, max_record_bytes: usize) -> io::Result<Self> {
        if !std::fs::metadata(path)?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "session is not a regular file",
            ));
        }
        let file = File::open(path)?;
        let m = file.metadata()?;
        if !m.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "session is not a regular file",
            ));
        }
        Ok(Self {
            path: path.to_owned(),
            file,
            byte_offset: 0,
            pending: Vec::new(),
            dropping: false,
            max_record_bytes: max_record_bytes.max(1),
            identity: FileIdentity::of(&m),
            modified: m.modified().ok(),
            prefix: Vec::new(),
        })
    }
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
    pub fn len(&self) -> io::Result<u64> {
        Ok(self.file.metadata()?.len())
    }
    pub fn is_empty(&self) -> io::Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Check pathname as well as the open handle: a rename can replace a growing file.
    pub fn check_reset(&mut self) -> io::Result<bool> {
        let m = std::fs::metadata(&self.path)?;
        let mut reset = FileIdentity::of(&m) != self.identity || m.len() < self.byte_offset;
        if !reset && m.modified().ok() != self.modified && !self.prefix.is_empty() {
            let mut file = File::open(&self.path)?;
            let mut prefix = vec![0; self.prefix.len()];
            reset = file.read_exact(&mut prefix).is_err() || prefix != self.prefix;
            // Equal-size rewrites cannot be append-only, even if the header is unchanged.
            reset |= m.len() == self.byte_offset;
        }
        if reset {
            *self = Self::open(&self.path, self.max_record_bytes)?;
        }
        self.modified = m.modified().ok();
        Ok(reset)
    }

    pub fn read_batch(
        &mut self,
        budget: usize,
        mut consume: impl FnMut(Record<'_>),
    ) -> io::Result<usize> {
        self.file.seek(SeekFrom::Start(self.byte_offset))?;
        let mut buf = [0_u8; 64 * 1024];
        let mut read = 0;
        while read < budget {
            let take = buf.len().min(budget - read);
            let n = self.file.read(&mut buf[..take])?;
            if n == 0 {
                break;
            }
            if self.prefix.len() < 128 {
                self.prefix
                    .extend_from_slice(&buf[..n.min(128 - self.prefix.len())]);
            }
            read += n;
            self.byte_offset += n as u64;
            let mut start = 0;
            while start < n {
                let end = memchr::memchr(b'\n', &buf[start..n]).map(|i| start + i);
                let stop = end.unwrap_or(n);
                if !self.dropping {
                    if self.pending.len() + stop - start > self.max_record_bytes {
                        self.pending.clear();
                        self.dropping = true;
                    } else {
                        self.pending.extend_from_slice(&buf[start..stop]);
                    }
                }
                if end.is_some() {
                    if self.dropping {
                        consume(Record::Oversized);
                    } else {
                        consume(Record::Line(&self.pending));
                    }
                    self.pending.clear();
                    self.dropping = false;
                    start = stop + 1;
                } else {
                    break;
                }
            }
        }
        self.modified = self.file.metadata()?.modified().ok();
        Ok(read)
    }
}
