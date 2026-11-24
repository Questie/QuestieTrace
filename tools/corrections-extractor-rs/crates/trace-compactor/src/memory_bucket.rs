//! A bucket in memory for tests, which can be made to stop answering at any request.

use crate::bucket::{md5_hex, Bucket, Listed};
use anyhow::{bail, Result};
use jiff::Timestamp;
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Mutex;

/// How the one failed request ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fault {
    /// The bucket did not carry the request out.
    Refused,
    /// The bucket carried the request out, and the answer never arrived.
    AnswerLost,
}

/// How an upload that is reported as stored went wrong.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PutFault {
    /// Nothing was stored under the key.
    NotStored,
    /// One byte of what was stored is not what was sent.
    Damaged,
}

pub struct MemoryBucket {
    state: Mutex<State>,
}

struct State {
    objects: BTreeMap<String, Object>,
    /// When a `put` through the trait says its object was stored.
    clock: Timestamp,
    /// Requests answered so far, through the trait.
    calls: usize,
    fault: Option<(usize, Fault)>,
    /// Set by the fault: every later request fails too, as for a process that died there.
    down: bool,
    /// Once this prefix has been listed, store these bytes under this key, or with None
    /// remove it.
    after_listing: Option<(String, String, Option<Vec<u8>>)>,
    /// When this key is next downloaded whole, store these bytes under it first.
    before_get: Option<(String, Vec<u8>)>,
    /// The next upload to a key with this prefix is reported as stored, and is not stored as
    /// it was sent.
    put_fault: Option<(String, PutFault)>,
}

struct Object {
    bytes: Vec<u8>,
    stored_at: Timestamp,
}

impl MemoryBucket {
    pub fn new(clock: Timestamp) -> Self {
        MemoryBucket {
            state: Mutex::new(State {
                objects: BTreeMap::new(),
                clock,
                calls: 0,
                fault: None,
                down: false,
                after_listing: None,
                before_get: None,
                put_fault: None,
            }),
        }
    }

    // ---- What a test does to the bucket directly: never counted, never failed ----

    pub fn store(&self, key: &str, bytes: &[u8], stored_at: Timestamp) {
        let object = Object {
            bytes: bytes.to_vec(),
            stored_at,
        };
        self.state.lock().unwrap().objects.insert(key.to_owned(), object);
    }

    pub fn remove(&self, key: &str) {
        self.state.lock().unwrap().objects.remove(key);
    }

    pub fn object(&self, key: &str) -> Option<Vec<u8>> {
        self.state.lock().unwrap().objects.get(key).map(|object| object.bytes.clone())
    }

    pub fn keys(&self, prefix: &str) -> Vec<String> {
        let state = self.state.lock().unwrap();
        state.objects.keys().filter(|key| key.starts_with(prefix)).cloned().collect()
    }

    /// Fails the request with this number, counted from zero from now on, and every request
    /// after it.
    pub fn fail_call(&self, call: usize, fault: Fault) {
        let mut state = self.state.lock().unwrap();
        state.calls = 0;
        state.fault = Some((call, fault));
    }

    /// Brings the bucket back. Reports whether the fault was reached.
    pub fn restore(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        state.fault = None;
        std::mem::take(&mut state.down)
    }

    /// Replaces an object the moment `prefix` has been listed: a change between a listing and
    /// the request that acts on it.
    pub fn after_listing(&self, prefix: &str, key: &str, bytes: &[u8]) {
        self.state.lock().unwrap().after_listing = Some((prefix.to_owned(), key.to_owned(), Some(bytes.to_vec())));
    }

    /// Removes an object the moment `prefix` has been listed.
    pub fn gone_after_listing(&self, prefix: &str, key: &str) {
        self.state.lock().unwrap().after_listing = Some((prefix.to_owned(), key.to_owned(), None));
    }

    /// Replaces an object the moment it is downloaded whole: a change between the reading of
    /// a part of it and the reading of all of it.
    pub fn before_get(&self, key: &str, bytes: &[u8]) {
        self.state.lock().unwrap().before_get = Some((key.to_owned(), bytes.to_vec()));
    }

    /// Makes the next upload to a key with this prefix go wrong without an error, as with a
    /// provider that did not check what it received.
    pub fn fail_put(&self, prefix: &str, fault: PutFault) {
        self.state.lock().unwrap().put_fault = Some((prefix.to_owned(), fault));
    }

    /// One request. `effect` reads or changes the objects and gives the answer.
    fn request<T>(&self, effect: impl FnOnce(&mut State) -> T) -> Result<T> {
        let mut state = self.state.lock().unwrap();
        if state.down {
            bail!("the bucket is down");
        }
        let call = state.calls;
        state.calls += 1;
        match state.fault {
            Some((at, fault)) if at == call => {
                state.down = true;
                if fault == Fault::AnswerLost {
                    effect(&mut state);
                }
                bail!("request {call} failed: {fault:?}")
            }
            _ => Ok(effect(&mut state)),
        }
    }
}

impl Bucket for MemoryBucket {
    fn list(&self, prefix: &str) -> Result<Vec<Listed>> {
        self.request(|state| {
            let listed = state
                .objects
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .map(|(key, object)| Listed {
                    key: key.clone(),
                    size: object.bytes.len() as u64,
                    etag: md5_hex(&object.bytes),
                    stored_at: object.stored_at,
                })
                .collect();
            if state.after_listing.as_ref().is_some_and(|(listed_prefix, _, _)| listed_prefix == prefix) {
                let (_, key, bytes) = state.after_listing.take().expect("checked above");
                let stored_at = state.clock;
                match bytes {
                    Some(bytes) => state.objects.insert(key, Object { bytes, stored_at }),
                    None => state.objects.remove(&key),
                };
            }
            listed
        })
    }

    fn etag(&self, key: &str) -> Result<Option<String>> {
        self.request(|state| state.objects.get(key).map(|object| md5_hex(&object.bytes)))
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.request(|state| {
            if state.before_get.as_ref().is_some_and(|(replaced, _)| replaced == key) {
                let (key, bytes) = state.before_get.take().expect("checked above");
                let stored_at = state.clock;
                state.objects.insert(key, Object { bytes, stored_at });
            }
            state.objects.get(key).map(|object| object.bytes.clone())
        })
    }

    fn get_range(&self, key: &str, range: Range<u64>) -> Result<Option<Vec<u8>>> {
        let part = self.request(|state| {
            let object = state.objects.get(key)?;
            Some(object.bytes.get(range.start as usize..range.end as usize).map(<[u8]>::to_vec))
        })?;
        match part {
            None => Ok(None),
            Some(Some(bytes)) => Ok(Some(bytes)),
            Some(None) => bail!("{key} does not hold bytes {}..{}", range.start, range.end),
        }
    }

    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        self.request(|state| {
            let mut bytes = bytes.to_vec();
            if state.put_fault.as_ref().is_some_and(|(prefix, _)| key.starts_with(prefix.as_str())) {
                match state.put_fault.take().expect("checked above").1 {
                    PutFault::NotStored => return,
                    PutFault::Damaged => {
                        let middle = bytes.len() / 2;
                        bytes[middle] ^= 1;
                    }
                }
            }
            let stored_at = state.clock;
            state.objects.insert(key.to_owned(), Object { bytes, stored_at });
        })
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.request(|state| {
            state.objects.remove(key);
        })
    }
}
