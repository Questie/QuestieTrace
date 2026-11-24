//! A bucket in memory for tests, which can be made to stop answering at any request.

use crate::bucket::{md5_hex, Bucket, Listed, Stat};
use anyhow::{bail, Result};
use jiff::Timestamp;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// How the one failed request ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fault {
    /// The bucket did not carry the request out.
    Refused,
    /// The bucket carried the request out, and the answer never arrived.
    AnswerLost,
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
    /// Once this prefix has been listed, store these bytes under this key.
    after_listing: Option<(String, String, Vec<u8>)>,
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

    /// Fails the request with this number, counted from zero, and every request after it.
    pub fn fail_call(&self, call: usize, fault: Fault) {
        self.state.lock().unwrap().fault = Some((call, fault));
    }

    /// Brings the bucket back and counts requests from zero again. Reports whether the fault
    /// was reached.
    pub fn restore(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        state.fault = None;
        state.calls = 0;
        std::mem::take(&mut state.down)
    }

    /// Replaces an object the moment `prefix` has been listed: a change between a listing and
    /// the request that acts on it.
    pub fn after_listing(&self, prefix: &str, key: &str, bytes: &[u8]) {
        self.state.lock().unwrap().after_listing = Some((prefix.to_owned(), key.to_owned(), bytes.to_vec()));
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
                state.objects.insert(key, Object { bytes, stored_at });
            }
            listed
        })
    }

    fn stat(&self, key: &str) -> Result<Option<Stat>> {
        self.request(|state| {
            state.objects.get(key).map(|object| Stat {
                size: object.bytes.len() as u64,
                etag: md5_hex(&object.bytes),
            })
        })
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.request(|state| state.objects.get(key).map(|object| object.bytes.clone()))
    }

    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        self.request(|state| {
            let object = Object {
                bytes: bytes.to_vec(),
                stored_at: state.clock,
            };
            state.objects.insert(key.to_owned(), object);
        })
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.request(|state| {
            state.objects.remove(key);
        })
    }
}
