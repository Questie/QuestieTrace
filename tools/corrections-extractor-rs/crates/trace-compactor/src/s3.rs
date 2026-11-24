//! The bucket on an S3-compatible provider.
//!
//! Every request is signed in its URL and sent as plain HTTP calls, one at a time on the
//! calling thread. A request that got no answer, or a server error, is tried again: every
//! request the compactor makes can be repeated.

use crate::bucket::{Bucket, Listed, Stat};
use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use md5::{Digest, Md5};
use rusty_s3::actions::ListObjectsV2;
use rusty_s3::{Credentials, S3Action, UrlStyle};
use std::time::Duration;
use ureq::http::Response;
use ureq::Body;

/// How long a signed URL is accepted. A request starts within moments of being signed.
const SIGNATURE_LIFETIME: Duration = Duration::from_secs(15 * 60);

const ATTEMPTS: u32 = 3;
const PAUSE_BETWEEN_ATTEMPTS: Duration = Duration::from_secs(2);

/// The longest one request may take, a pack upload included. Without it a connection that
/// stalls would hold the run, and the timer that starts the next one, for good.
const LONGEST_REQUEST: Duration = Duration::from_secs(30 * 60);

/// The most a download may hold. A single PUT, which is how every object here was stored,
/// cannot exceed 5 GiB.
const LARGEST_OBJECT: u64 = 5 << 30;

pub struct S3Bucket {
    bucket: rusty_s3::Bucket,
    credentials: Credentials,
    agent: ureq::Agent,
}

/// Why one attempt at a request failed.
enum Failure {
    /// No answer, or a server error: the request may be sent again.
    Unanswered(anyhow::Error),
    /// The provider's answer. Sending the request again would get it again.
    Refused(anyhow::Error),
}

impl S3Bucket {
    /// `endpoint` is the provider's URL, such as `https://s3.uk.io.cloud.ovh.net`. The bucket
    /// is addressed in the path, which every S3-compatible provider accepts.
    pub fn new(endpoint: &str, region: &str, name: &str, access_key: &str, secret_key: &str) -> Result<Self> {
        let endpoint = endpoint.parse().with_context(|| format!("{endpoint} is not a URL"))?;
        let bucket = rusty_s3::Bucket::new(endpoint, UrlStyle::Path, name.to_owned(), region.to_owned()).context("the bucket's address")?;
        let agent = ureq::Agent::config_builder()
            // Statuses are read here: 404 is an answer, not a failure.
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(LONGEST_REQUEST))
            .build()
            .new_agent();
        Ok(S3Bucket {
            bucket,
            credentials: Credentials::new(access_key, secret_key),
            agent,
        })
    }

    /// Runs `attempt` until it succeeds, is refused, or has failed [`ATTEMPTS`] times.
    fn retrying<T>(&self, mut attempt: impl FnMut() -> Result<T, Failure>) -> Result<T> {
        for tried in 1.. {
            match attempt() {
                Ok(answer) => return Ok(answer),
                Err(Failure::Refused(error)) => return Err(error),
                Err(Failure::Unanswered(error)) if tried == ATTEMPTS => return Err(error),
                Err(Failure::Unanswered(_)) => std::thread::sleep(PAUSE_BETWEEN_ATTEMPTS * tried),
            }
        }
        unreachable!("the loop returns")
    }

    /// A request that ended without the provider's answer.
    fn unanswered(&self, operation: &str, key: &str, error: ureq::Error) -> Failure {
        // The URL carries the request's signature; an error that quotes it is not repeated.
        let mut reason = error.to_string();
        if reason.contains("X-Amz-") {
            reason = "the request failed".into();
        }
        Failure::Unanswered(anyhow!("bucket {}: {operation} of {key}: {reason}", self.bucket.name()))
    }

    /// An answer that is not the one the operation needs. The provider's error code is kept
    /// for diagnosis; its message, which can repeat request details, is not.
    fn unexpected(&self, operation: &str, key: &str, mut response: Response<Body>) -> Failure {
        let status = response.status().as_u16();
        let body = response.body_mut().with_config().limit(64 << 10).read_to_string().unwrap_or_default();
        let code = body
            .split_once("<Code>")
            .and_then(|(_, rest)| rest.split_once("</Code>"))
            .map_or("", |(code, _)| code);
        let error = anyhow!("bucket {}: {operation} of {key} returned HTTP {status} {code}", self.bucket.name());
        if status >= 500 || status == 429 {
            Failure::Unanswered(error)
        } else {
            Failure::Refused(error)
        }
    }

    /// Stores `bytes` under `key` if they have this MD5, given in base64.
    fn put_with_md5(&self, key: &str, bytes: &[u8], md5: &str) -> Result<()> {
        self.retrying(|| {
            let mut action = self.bucket.put_object(Some(&self.credentials), key);
            action.headers_mut().insert("content-md5", md5);
            let url = action.sign(SIGNATURE_LIFETIME);
            let sent = self.agent.put(url.as_str()).header("content-md5", md5).send(bytes);
            let response = sent.map_err(|error| self.unanswered("upload", key, error))?;
            match response.status().as_u16() {
                200 => Ok(()),
                _ => Err(self.unexpected("upload", key, response)),
            }
        })
    }

    /// One page of a listing, and the token of the next page when there is one.
    fn list_page(&self, prefix: &str, continuation: Option<&str>) -> Result<(Vec<Listed>, Option<String>)> {
        self.retrying(|| {
            let mut action = self.bucket.list_objects_v2(Some(&self.credentials));
            action.with_prefix(prefix);
            if let Some(token) = continuation {
                action.with_continuation_token(token);
            }
            let url = action.sign(SIGNATURE_LIFETIME);
            let mut response = self.agent.get(url.as_str()).call().map_err(|error| self.unanswered("listing", prefix, error))?;
            if response.status() != 200 {
                return Err(self.unexpected("listing", prefix, response));
            }
            let text = response
                .body_mut()
                .with_config()
                .limit(64 << 20)
                .read_to_string()
                .map_err(|error| self.unanswered("listing", prefix, error))?;

            // A page that cannot be read whole is no page: a listing is all of the objects or
            // an error.
            let unreadable = |what: &str| Failure::Refused(anyhow!("bucket {}: listing of {prefix}: {what}", self.bucket.name()));
            let page = ListObjectsV2::parse_response(&text).map_err(|error| unreadable(&error.to_string()))?;
            let mut listed = Vec::with_capacity(page.contents.len());
            for object in page.contents {
                let (Some(size), Some(etag), Some(stored_at)) = (object.size, &object.etag, &object.last_modified) else {
                    return Err(unreadable("an object is listed without its size, ETag or time"));
                };
                listed.push(Listed {
                    size,
                    etag: etag.trim_matches('"').to_owned(),
                    stored_at: stored_at.parse().map_err(|_| unreadable("an object's time is not a timestamp"))?,
                    key: object.key,
                });
            }
            Ok((listed, page.next_continuation_token))
        })
    }
}

impl Bucket for S3Bucket {
    fn list(&self, prefix: &str) -> Result<Vec<Listed>> {
        let (mut listed, mut next) = self.list_page(prefix, None)?;
        while let Some(token) = next {
            let (page, following) = self.list_page(prefix, Some(&token))?;
            listed.extend(page);
            next = following;
        }
        Ok(listed)
    }

    fn stat(&self, key: &str) -> Result<Option<Stat>> {
        self.retrying(|| {
            let url = self.bucket.head_object(Some(&self.credentials), key).sign(SIGNATURE_LIFETIME);
            let response = self.agent.head(url.as_str()).call().map_err(|error| self.unanswered("check", key, error))?;
            match response.status().as_u16() {
                200 => {
                    let header = |name: &str| response.headers().get(name).and_then(|value| value.to_str().ok());
                    let size = header("content-length").and_then(|length| length.parse().ok());
                    let etag = header("etag").map(|etag| etag.trim_matches('"').to_owned()).filter(|etag| !etag.is_empty());
                    match (size, etag) {
                        (Some(size), Some(etag)) => Ok(Some(Stat { size, etag })),
                        _ => Err(Failure::Refused(anyhow!(
                            "bucket {}: check of {key} returned no size or no ETag",
                            self.bucket.name()
                        ))),
                    }
                }
                404 => Ok(None),
                _ => Err(self.unexpected("check", key, response)),
            }
        })
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.retrying(|| {
            let url = self.bucket.get_object(Some(&self.credentials), key).sign(SIGNATURE_LIFETIME);
            let mut response = self.agent.get(url.as_str()).call().map_err(|error| self.unanswered("download", key, error))?;
            match response.status().as_u16() {
                200 => {
                    let body = response.body_mut().with_config().limit(LARGEST_OBJECT).read_to_vec();
                    body.map(Some).map_err(|error| self.unanswered("download", key, error))
                }
                404 => Ok(None),
                _ => Err(self.unexpected("download", key, response)),
            }
        })
    }

    /// The MD5 is part of what is signed and sent, so the provider refuses a body that
    /// arrived damaged.
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        self.put_with_md5(key, bytes, &BASE64.encode(Md5::digest(bytes)))
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.retrying(|| {
            let url = self.bucket.delete_object(Some(&self.credentials), key).sign(SIGNATURE_LIFETIME);
            let response = self
                .agent
                .delete(url.as_str())
                .call()
                .map_err(|error| self.unanswered("deletion", key, error))?;
            match response.status().as_u16() {
                200 | 204 | 404 => Ok(()),
                _ => Err(self.unexpected("deletion", key, response)),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bucket::{md5_hex, LOOSE};
    use crate::packing::{run, Config};
    use jiff::{SignedDuration, Timestamp};
    use trace_pack::pack::{PackOptions, PackReader};

    /// A disposable S3 server on plain HTTP, for example:
    ///
    /// ```text
    /// docker run --rm -p 127.0.0.1:19000:9000 -e RUSTFS_ACCESS_KEY=questie-test \
    ///   -e RUSTFS_SECRET_KEY=questie-test-secret rustfs/rustfs
    /// TRACE_COMPACTOR_TEST_S3=http://127.0.0.1:19000 cargo test -p trace-compactor s3
    /// ```
    ///
    /// Each test creates a bucket of its own there, `compactor-test-<name>`, and leaves it
    /// empty.
    fn test_server(name: &str) -> Option<S3Bucket> {
        let endpoint = std::env::var("TRACE_COMPACTOR_TEST_S3").ok()?;
        let bucket = S3Bucket::new(&endpoint, "us-east-1", &format!("compactor-test-{name}"), "questie-test", "questie-test-secret").unwrap();
        let create = rusty_s3::actions::CreateBucket::new(&bucket.bucket, &bucket.credentials).sign(SIGNATURE_LIFETIME);
        let status = bucket.agent.put(create.as_str()).send_empty().unwrap().status().as_u16();
        assert!(status == 200 || status == 409, "creating the test bucket returned HTTP {status}");
        for object in bucket.list("").unwrap() {
            bucket.delete(&object.key).unwrap();
        }
        Some(bucket)
    }

    /// The provider must answer each request the way the compactor reads it: an ETag that is
    /// the MD5, an absent key as absent and not as an error, a listing limited to its prefix.
    #[test]
    fn s3_server_answers_as_the_compactor_reads_it() {
        let Some(bucket) = test_server("answers") else {
            eprintln!("skipped: set TRACE_COMPACTOR_TEST_S3 to a disposable S3 server");
            return;
        };
        let before = Timestamp::now().checked_sub(SignedDuration::from_secs(120)).unwrap();
        let content = b"{\"export_string\": \"\\u0000\"}\n";
        bucket.put("check/a b+c.json", content).unwrap();
        bucket.put("check/empty", b"").unwrap();
        bucket.put("other/x", b"x").unwrap();

        assert_eq!(bucket.get("check/a b+c.json").unwrap().as_deref(), Some(&content[..]));
        let stat = bucket.stat("check/a b+c.json").unwrap().unwrap();
        assert_eq!((stat.size, stat.etag.as_str()), (content.len() as u64, md5_hex(content).as_str()));
        assert_eq!(bucket.stat("check/empty").unwrap().unwrap().size, 0);

        let mut listed = bucket.list("check/").unwrap();
        listed.sort_by(|a, b| a.key.cmp(&b.key));
        let keys: Vec<&str> = listed.iter().map(|object| object.key.as_str()).collect();
        assert_eq!(keys, ["check/a b+c.json", "check/empty"]);
        assert_eq!((listed[0].size, listed[0].etag.as_str()), (content.len() as u64, md5_hex(content).as_str()));
        let after = Timestamp::now().checked_add(SignedDuration::from_secs(120)).unwrap();
        assert!(
            before <= listed[0].stored_at && listed[0].stored_at <= after,
            "stored at {}",
            listed[0].stored_at
        );

        assert_eq!(bucket.get("check/absent").unwrap(), None);
        assert_eq!(bucket.stat("check/absent").unwrap(), None);
        bucket.delete("check/absent").unwrap();
        for key in ["check/a b+c.json", "check/empty", "other/x"] {
            bucket.delete(key).unwrap();
            assert_eq!(bucket.stat(key).unwrap(), None);
        }
        assert_eq!(bucket.list("").unwrap(), []);
    }

    #[test]
    fn s3_listing_follows_every_page() {
        let Some(bucket) = test_server("pages") else {
            return;
        };
        // A page holds at most 1,000 keys.
        let keys: Vec<String> = (0..1005).map(|index| format!("many/{index:04}")).collect();
        rayon::scope(|scope| {
            for chunk in keys.chunks(100) {
                let bucket = &bucket;
                scope.spawn(move |_| chunk.iter().for_each(|key| bucket.put(key, b"x").unwrap()));
            }
        });
        let mut listed: Vec<String> = bucket.list("many/").unwrap().into_iter().map(|object| object.key).collect();
        listed.sort();
        assert_eq!(listed, keys);
        rayon::scope(|scope| {
            for chunk in keys.chunks(100) {
                let bucket = &bucket;
                scope.spawn(move |_| chunk.iter().for_each(|key| bucket.delete(key).unwrap()));
            }
        });
    }

    #[test]
    fn s3_refuses_wrong_credentials_without_retrying_into_success() {
        let Some(bucket) = test_server("credentials") else {
            return;
        };
        let endpoint = std::env::var("TRACE_COMPACTOR_TEST_S3").unwrap();
        let stranger = S3Bucket::new(&endpoint, "us-east-1", bucket.bucket.name(), "questie-test", "not-the-secret-key").unwrap();
        bucket.put("check/kept", b"kept").unwrap();
        let refused = format!("{:#}", stranger.put("check/kept", b"overwritten").unwrap_err());
        assert!(refused.contains("HTTP 403"), "{refused}");
        assert!(!refused.contains("X-Amz-"), "{refused}");
        stranger.list("check/").unwrap_err();
        stranger.delete("check/kept").unwrap_err();
        assert_eq!(bucket.get("check/kept").unwrap().as_deref(), Some(&b"kept"[..]));
        bucket.delete("check/kept").unwrap();
    }

    /// A pack is uploaded with its MD5 so that the provider refuses one damaged on the way.
    /// That holds only if the provider checks it.
    #[test]
    fn s3_refuses_a_body_that_is_not_what_its_md5_says() {
        let Some(bucket) = test_server("md5") else {
            return;
        };
        let md5_of_other_bytes = BASE64.encode(Md5::digest(b"what was meant to be sent"));
        let refused = format!("{:#}", bucket.put_with_md5("check/damaged", b"what arrived", &md5_of_other_bytes).unwrap_err());
        assert!(refused.contains("HTTP 400"), "{refused}");
        assert_eq!(bucket.stat("check/damaged").unwrap(), None);
    }

    #[test]
    fn s3_run_packs_and_deletes_loose_objects() {
        let Some(bucket) = test_server("run") else {
            return;
        };
        let name = "2026-11/2026-11-08_180449Z_0123456789abcdef0123456789abcdef.json";
        let file = br#"{"schema_version":2,"id":"0123456789abcdef0123456789abcdef","export_string":"not one"}"#;
        bucket.put(&format!("{LOOSE}{name}"), file).unwrap();
        let config = Config {
            writer: "live".into(),
            settle: SignedDuration::ZERO,
            pack_input_bytes: u64::MAX,
            options: PackOptions::default(),
        };
        // The server's clock may be a little ahead of this machine's.
        let now = Timestamp::now().checked_add(SignedDuration::from_secs(120)).unwrap();

        let reports = run(&bucket, &config, now).unwrap();

        let keys: Vec<String> = bucket.list("").unwrap().into_iter().map(|object| object.key).collect();
        assert_eq!(keys, [reports[0].key.clone()]);
        let pack = bucket.get(&reports[0].key).unwrap().unwrap();
        let reader = PackReader::open(&pack).unwrap();
        assert_eq!(reader.entries()[0].name, name);
        bucket.delete(&reports[0].key).unwrap();
    }
    /// Real submissions through the whole run: set TRACE_COMPACTOR_TEST_SUBMISSIONS to a month
    /// of a trace-data checkout, such as `.../trace-data/submissions/2026-11`. Every record of
    /// every pack is then compared with its file by the pack format's own check.
    #[test]
    fn s3_run_packs_real_submissions_that_read_back_as_their_files() {
        let (Some(bucket), Ok(month)) = (test_server("real"), std::env::var("TRACE_COMPACTOR_TEST_SUBMISSIONS")) else {
            return;
        };
        let month = std::path::PathBuf::from(month);
        let mut names: Vec<String> = std::fs::read_dir(&month)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".json"))
            .collect();
        names.sort();
        names.truncate(40);
        let folder = month.file_name().unwrap().to_str().unwrap();
        let mut loose_bytes = 0;
        for name in &names {
            let file = std::fs::read(month.join(name)).unwrap();
            loose_bytes += file.len() as u64;
            bucket.put(&format!("{LOOSE}{folder}/{name}"), &file).unwrap();
        }
        let config = Config {
            writer: "live".into(),
            settle: SignedDuration::ZERO,
            // Several packs, so the run goes round more than once.
            pack_input_bytes: loose_bytes / 3,
            options: PackOptions::default(),
        };
        let now = Timestamp::now().checked_add(SignedDuration::from_secs(120)).unwrap();

        let reports = run(&bucket, &config, now).unwrap();

        assert!(reports.len() >= 3, "{reports:?}");
        let mut scratch = trace_pack::pack::Scratch::new();
        let mut packed = Vec::new();
        for report in &reports {
            assert_eq!(report.value_fallbacks, [] as [String; 0]);
            let pack = bucket.get(&report.key).unwrap().unwrap();
            let reader = PackReader::open(&pack).unwrap();
            for (index, entry) in reader.entries().iter().enumerate() {
                let file = std::fs::read(month.join(entry.name.strip_prefix(&format!("{folder}/")).unwrap())).unwrap();
                trace_pack::pack::check_against_file(&reader.record(index).unwrap(), entry, &file, &config.options.tp, &mut scratch)
                    .unwrap_or_else(|message| panic!("{}: {message}", entry.name));
                packed.push(entry.name.clone());
            }
            bucket.delete(&report.key).unwrap();
        }
        packed.sort();
        assert_eq!(packed, names.iter().map(|name| format!("{folder}/{name}")).collect::<Vec<_>>());
        assert_eq!(bucket.list("").unwrap(), []);
        let pack_bytes: u64 = reports.iter().map(|report| report.pack_bytes).sum();
        eprintln!(
            "{} real submissions: {loose_bytes} bytes of JSON in {} packs of {pack_bytes} bytes",
            names.len(),
            reports.len()
        );
    }
}
