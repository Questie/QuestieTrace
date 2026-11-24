//! The bucket on an S3-compatible provider.
//!
//! Every request is signed in its URL and sent as a plain HTTP call, on the calling thread.
//! A request that got no answer, or a server error for one, is sent again: every request the
//! compactor makes can be repeated.

use crate::bucket::{Bucket, Listed};
use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use md5::{Digest, Md5};
use rusty_s3::actions::ListObjectsV2;
use rusty_s3::{Credentials, S3Action, UrlStyle};
use std::ops::Range;
use std::time::Duration;
use ureq::http::Response;
use ureq::Body;

/// How long a signed URL is accepted. A request starts within moments of being signed.
const SIGNATURE_LIFETIME: Duration = Duration::from_secs(15 * 60);

const ATTEMPTS: u32 = 3;

/// The longest one request may take, a pack upload included. Without it a connection that
/// stalls would hold the run, and the timer that starts the next one, for good.
const LONGEST_REQUEST: Duration = Duration::from_secs(30 * 60);

/// The most a download may hold: the largest object one PUT can store. The compactor's packs
/// are stored that way, and a loose object is a copy of one that was.
const LARGEST_OBJECT: u64 = 5 << 30;

pub struct S3Bucket {
    bucket: rusty_s3::Bucket,
    credentials: Credentials,
    agent: ureq::Agent,
    /// Between two attempts at one request, times the number of attempts made.
    pause: Duration,
}

/// Why one attempt at a request failed.
#[derive(Debug)]
enum Failure {
    /// No answer, or the provider's own trouble: the request may be sent again.
    Transient(anyhow::Error),
    /// The provider's answer to this request. Sending it again would get the same.
    Final(anyhow::Error),
}

/// Runs `attempt` until it succeeds, fails for good, or has failed [`ATTEMPTS`] times.
fn retrying<T>(pause: Duration, mut attempt: impl FnMut() -> Result<T, Failure>) -> Result<T> {
    for tried in 1.. {
        match attempt() {
            Ok(answer) => return Ok(answer),
            Err(Failure::Final(error)) => return Err(error),
            Err(Failure::Transient(error)) if tried == ATTEMPTS => return Err(error),
            Err(Failure::Transient(_)) => std::thread::sleep(pause * tried),
        }
    }
    unreachable!("the loop returns")
}

/// Whether a status is the provider's trouble and not its answer: a server error, or a
/// request to slow down.
fn is_transient(status: u16) -> bool {
    status >= 500 || status == 429
}

/// What to say about a request that got no answer. The URL carries the request's signature,
/// so an error that quotes it is not repeated.
fn without_signature(reason: String) -> String {
    if reason.contains("X-Amz-") {
        "the request failed".into()
    } else {
        reason
    }
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
            // A redirect is followed as a GET, which would turn an upload or a deletion into
            // a download that reports success.
            .max_redirects(0)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(LONGEST_REQUEST))
            .build()
            .new_agent();
        Ok(S3Bucket {
            bucket,
            credentials: Credentials::new(access_key, secret_key),
            agent,
            pause: Duration::from_secs(2),
        })
    }

    /// A request that ended without the provider's answer.
    fn unanswered(&self, operation: &str, key: &str, error: ureq::Error) -> Failure {
        let reason = without_signature(error.to_string());
        Failure::Transient(anyhow!("bucket {}: {operation} of {key}: {reason}", self.bucket.name()))
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
        if is_transient(status) {
            Failure::Transient(error)
        } else {
            Failure::Final(error)
        }
    }

    /// One page of a listing, and the token of the next page when there is one.
    fn list_page(&self, prefix: &str, continuation: Option<&str>) -> Result<(Vec<Listed>, Option<String>)> {
        retrying(self.pause, || {
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
            let text = response.body_mut().with_config().limit(64 << 20).read_to_string();
            let text = text.map_err(|error| self.unanswered("listing", prefix, error))?;

            // A page that cannot be read whole is no page: a listing is all of the objects or
            // an error.
            let unreadable = |what: &str| Failure::Final(anyhow!("bucket {}: listing of {prefix}: {what}", self.bucket.name()));
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

    /// Stores `bytes` under `key` if they have this MD5, given in base64. The MD5 is part of
    /// what is signed, and a provider that keeps to S3 refuses a body that does not match it.
    fn put_with_md5(&self, key: &str, bytes: &[u8], md5: &str) -> Result<()> {
        retrying(self.pause, || {
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

    fn etag(&self, key: &str) -> Result<Option<String>> {
        retrying(self.pause, || {
            let url = self.bucket.head_object(Some(&self.credentials), key).sign(SIGNATURE_LIFETIME);
            let response = self.agent.head(url.as_str()).call().map_err(|error| self.unanswered("check", key, error))?;
            match response.status().as_u16() {
                200 => {
                    let etag = response.headers().get("etag").and_then(|value| value.to_str().ok());
                    match etag.map(|etag| etag.trim_matches('"')).filter(|etag| !etag.is_empty()) {
                        Some(etag) => Ok(Some(etag.to_owned())),
                        // An object without one could never be confirmed or deleted.
                        None => Err(Failure::Final(anyhow!("bucket {}: check of {key} returned no ETag", self.bucket.name()))),
                    }
                }
                404 => Ok(None),
                _ => Err(self.unexpected("check", key, response)),
            }
        })
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        retrying(self.pause, || {
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

    fn get_range(&self, key: &str, range: Range<u64>) -> Result<Option<Vec<u8>>> {
        let wanted = range.end.saturating_sub(range.start);
        if wanted == 0 {
            bail!("bucket {}: download of no bytes of {key}", self.bucket.name());
        }
        // HTTP names the last byte, not the one after it.
        let header = format!("bytes={}-{}", range.start, range.end - 1);
        retrying(self.pause, || {
            let url = self.bucket.get_object(Some(&self.credentials), key).sign(SIGNATURE_LIFETIME);
            let sent = self.agent.get(url.as_str()).header("range", &header).call();
            let mut response = sent.map_err(|error| self.unanswered("download", key, error))?;
            match response.status().as_u16() {
                206 => {
                    let body = response.body_mut().with_config().limit(wanted + 1).read_to_vec();
                    let bytes = body.map_err(|error| self.unanswered("download", key, error))?;
                    // A range that reaches past the object's end is answered with what there is.
                    if bytes.len() as u64 != wanted {
                        return Err(Failure::Final(anyhow!(
                            "bucket {}: download of {header} of {key} returned {} bytes",
                            self.bucket.name(),
                            bytes.len()
                        )));
                    }
                    Ok(Some(bytes))
                }
                404 => Ok(None),
                _ => Err(self.unexpected("download", key, response)),
            }
        })
    }

    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        self.put_with_md5(key, bytes, &BASE64.encode(Md5::digest(bytes)))
    }

    fn delete(&self, key: &str) -> Result<()> {
        retrying(self.pause, || {
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
    use crate::{run, Config};
    use jiff::{SignedDuration, Timestamp};
    use trace_pack::pack::{PackOptions, PackReader};

    // ---- Without a server ----

    #[test]
    fn a_request_is_sent_again_only_while_that_can_help() {
        let attempts = |outcomes: &[fn() -> Result<u32, Failure>]| {
            let mut made = 0;
            let result = retrying(Duration::ZERO, || {
                made += 1;
                outcomes[made - 1]()
            });
            (made, result.map_err(|error| error.to_string()))
        };
        let transient: fn() -> Result<u32, Failure> = || Err(Failure::Transient(anyhow!("no answer")));
        let fatal: fn() -> Result<u32, Failure> = || Err(Failure::Final(anyhow!("refused")));
        let fine: fn() -> Result<u32, Failure> = || Ok(7);

        assert_eq!(attempts(&[fine]), (1, Ok(7)));
        assert_eq!(attempts(&[transient, transient, fine]), (3, Ok(7)));
        // Three attempts, then the last failure is the run's.
        assert_eq!(attempts(&[transient, transient, transient, fine]), (3, Err("no answer".into())));
        // A refusal would be refused again.
        assert_eq!(attempts(&[fatal, fine]), (1, Err("refused".into())));
    }

    #[test]
    fn only_the_providers_own_trouble_is_worth_another_attempt() {
        for status in [500, 502, 503, 504, 429] {
            assert!(is_transient(status), "{status}");
        }
        // Each of these is an answer about this request: wrong key, wrong body, no such
        // bucket, a redirect that is not followed.
        for status in [301, 307, 400, 403, 404, 409, 412, 416] {
            assert!(!is_transient(status), "{status}");
        }
    }

    #[test]
    fn an_error_that_quotes_the_signed_url_is_not_repeated() {
        let quoting = "http: bad uri https://s3.example/b/k?X-Amz-Credential=AKIA&X-Amz-Signature=abc".to_owned();
        assert_eq!(without_signature(quoting), "the request failed");
        assert_eq!(without_signature("io: connection reset".into()), "io: connection reset");
    }

    // ---- Against a server ----
    //
    // These need an S3 server and are skipped unless asked for with `--ignored`.
    //
    // A disposable one, on plain HTTP, runs all of them. Each test creates a bucket of its
    // own there, `compactor-test-<name>`, and leaves it empty:
    //
    //   docker run --rm -p 127.0.0.1:19000:9000 -e RUSTFS_ACCESS_KEY=questie-test \
    //     -e RUSTFS_SECRET_KEY=questie-test-secret rustfs/rustfs
    //   TRACE_COMPACTOR_TEST_S3=http://127.0.0.1:19000 cargo test -p trace-compactor s3 -- --ignored
    //
    // The tests named `provider_...` also run against a real provider, to see that it
    // answers the way the compactor reads it. They use the bucket of the ARCHIVE_S3_*
    // variables the compactor itself takes, and touch only keys under
    // `compactor-check/<test>-<time>/`, which they delete again:
    //
    //   TRACE_COMPACTOR_TEST_S3=provider cargo test -p trace-compactor provider -- --ignored

    const NEEDS_A_SERVER: &str = "set TRACE_COMPACTOR_TEST_S3: see the comment above these tests";

    /// A bucket of the disposable server that is this test's alone, emptied.
    fn disposable(name: &str) -> S3Bucket {
        let endpoint = std::env::var("TRACE_COMPACTOR_TEST_S3").expect(NEEDS_A_SERVER);
        assert!(
            endpoint != "provider",
            "this test needs a bucket of its own: run it against a disposable server"
        );
        let bucket = S3Bucket::new(&endpoint, "us-east-1", &format!("compactor-test-{name}"), "questie-test", "questie-test-secret").unwrap();
        let create = rusty_s3::actions::CreateBucket::new(&bucket.bucket, &bucket.credentials).sign(SIGNATURE_LIFETIME);
        let status = bucket.agent.put(create.as_str()).send_empty().unwrap().status().as_u16();
        assert!(status == 200 || status == 409, "creating the test bucket returned HTTP {status}");
        for object in bucket.list("").unwrap() {
            bucket.delete(&object.key).unwrap();
        }
        bucket
    }

    /// A bucket and a key prefix nothing else uses: on the disposable server or, with
    /// `TRACE_COMPACTOR_TEST_S3=provider`, in the bucket the environment names.
    fn provider(name: &str) -> (S3Bucket, String) {
        if std::env::var("TRACE_COMPACTOR_TEST_S3").expect(NEEDS_A_SERVER) != "provider" {
            return (disposable(name), "check/".into());
        }
        let variable = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
        let bucket = S3Bucket::new(
            &format!("https://{}", variable("ARCHIVE_S3_ENDPOINT")),
            &variable("ARCHIVE_S3_REGION"),
            &variable("ARCHIVE_S3_BUCKET"),
            &variable("ARCHIVE_S3_ACCESS_KEY_ID"),
            &variable("ARCHIVE_S3_SECRET_ACCESS_KEY"),
        )
        .unwrap();
        (bucket, format!("compactor-check/{name}-{}/", Timestamp::now().as_millisecond()))
    }

    /// The provider must answer each request the way the compactor reads it: an ETag that is
    /// the MD5, an absent key as absent and not as an error, a listing limited to its prefix.
    #[test]
    #[ignore = "needs an S3 server"]
    fn provider_answers_as_the_compactor_reads_it() {
        let (bucket, prefix) = provider("answers");
        let key = |name: &str| format!("{prefix}{name}");
        let before = Timestamp::now().checked_sub(SignedDuration::from_secs(120)).unwrap();
        let content = b"{\"export_string\": \"\\u0000\"}\n";
        bucket.put(&key("in/a b+c.json"), content).unwrap();
        bucket.put(&key("in/empty"), b"").unwrap();
        bucket.put(&key("other/x"), b"x").unwrap();

        assert_eq!(bucket.get(&key("in/a b+c.json")).unwrap().as_deref(), Some(&content[..]));
        assert_eq!(bucket.etag(&key("in/a b+c.json")).unwrap(), Some(md5_hex(content)));
        assert_eq!(bucket.etag(&key("in/empty")).unwrap(), Some(md5_hex(b"")));

        let mut listed = bucket.list(&key("in/")).unwrap();
        listed.sort_by(|a, b| a.key.cmp(&b.key));
        let keys: Vec<&str> = listed.iter().map(|object| object.key.as_str()).collect();
        assert_eq!(keys, [key("in/a b+c.json"), key("in/empty")]);
        assert_eq!((listed[0].size, listed[0].etag.as_str()), (content.len() as u64, md5_hex(content).as_str()));
        assert_eq!(listed[1].size, 0);
        let after = Timestamp::now().checked_add(SignedDuration::from_secs(120)).unwrap();
        assert!(
            before <= listed[0].stored_at && listed[0].stored_at <= after,
            "stored at {}",
            listed[0].stored_at
        );

        // A part of an object: exactly the bytes asked for, and an error when it does not
        // hold them all.
        assert_eq!(bucket.get_range(&key("in/a b+c.json"), 1..4).unwrap().as_deref(), Some(&content[1..4]));
        let whole = 0..content.len() as u64;
        assert_eq!(bucket.get_range(&key("in/a b+c.json"), whole.clone()).unwrap().as_deref(), Some(&content[..]));
        bucket.get_range(&key("in/a b+c.json"), 1..whole.end + 1).unwrap_err();
        bucket.get_range(&key("in/a b+c.json"), whole.end + 5..whole.end + 9).unwrap_err();
        assert_eq!(bucket.get_range(&key("in/absent"), 0..4).unwrap(), None);

        // A replaced object has the new content's ETag.
        bucket.put(&key("other/x"), b"y").unwrap();
        assert_eq!(bucket.etag(&key("other/x")).unwrap(), Some(md5_hex(b"y")));

        assert_eq!(bucket.get(&key("in/absent")).unwrap(), None);
        assert_eq!(bucket.etag(&key("in/absent")).unwrap(), None);
        bucket.delete(&key("in/absent")).unwrap();
        for name in ["in/a b+c.json", "in/empty", "other/x"] {
            bucket.delete(&key(name)).unwrap();
            assert_eq!(bucket.etag(&key(name)).unwrap(), None);
        }
        assert_eq!(bucket.list(&prefix).unwrap(), []);
    }

    #[test]
    #[ignore = "needs an S3 server"]
    fn provider_listing_follows_every_page() {
        let (bucket, prefix) = provider("pages");
        // A page holds at most 1,000 keys.
        let keys: Vec<String> = (0..1005).map(|index| format!("{prefix}{index:04}")).collect();
        let on_every = |action: &(dyn Fn(&String) + Sync)| {
            rayon::scope(|scope| keys.chunks(100).for_each(|chunk| scope.spawn(move |_| chunk.iter().for_each(action))));
        };
        on_every(&|key| bucket.put(key, b"x").unwrap());
        let mut listed: Vec<String> = bucket.list(&prefix).unwrap().into_iter().map(|object| object.key).collect();
        listed.sort();
        on_every(&|key| bucket.delete(key).unwrap());
        assert_eq!(listed, keys);
    }

    /// A pack is uploaded with its MD5 so that the provider refuses one damaged on the way.
    /// That holds only if the provider checks it.
    #[test]
    #[ignore = "needs an S3 server"]
    fn provider_refuses_a_body_that_is_not_what_its_md5_says() {
        let (bucket, prefix) = provider("md5");
        let key = format!("{prefix}damaged");
        let md5_of_other_bytes = BASE64.encode(Md5::digest(b"what was meant to be sent"));
        let refused = format!("{:#}", bucket.put_with_md5(&key, b"what arrived", &md5_of_other_bytes).unwrap_err());
        assert!(refused.contains("HTTP 400"), "{refused}");
        assert_eq!(bucket.etag(&key).unwrap(), None);
    }

    #[test]
    #[ignore = "needs an S3 server"]
    fn s3_refuses_wrong_credentials_and_says_so_without_the_signature() {
        let bucket = disposable("credentials");
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

    fn pack_everything() -> Config {
        Config {
            writer: "live".into(),
            settle: SignedDuration::ZERO,
            pack_input_bytes: u64::MAX,
            build_input_bytes: u64::MAX,
            options: PackOptions::default(),
        }
    }

    /// A moment by which everything just stored has settled, whatever the server's clock
    /// says against this machine's.
    fn in_two_minutes() -> Timestamp {
        Timestamp::now().checked_add(SignedDuration::from_secs(120)).unwrap()
    }

    /// A whole run against a real server, removal included: the indexes are read through
    /// ranged downloads, and the pack is replaced under its own key.
    #[test]
    #[ignore = "needs an S3 server"]
    fn s3_run_packs_loose_objects_and_then_removes_one_from_the_pack() {
        let bucket = disposable("run");
        let file = |id: &str| format!(r#"{{"schema_version":2,"id":"{id}","export_string":"not one"}}"#).into_bytes();
        let name = |id: &str| format!("2026-11/2026-11-08_180449Z_{id}.json");
        for id in ["aaaa", "bbbb", "cccc"] {
            bucket.put(&format!("{LOOSE}{}", name(id)), &file(id)).unwrap();
        }

        let packed = run(&bucket, &pack_everything(), in_two_minutes()).unwrap().packed;

        let pack_key = packed.packs[0].key.clone();
        let keys: Vec<String> = bucket.list("").unwrap().into_iter().map(|object| object.key).collect();
        assert_eq!(keys, [pack_key.as_str()]);

        bucket
            .put("removals/requested/bbbb", br#"{"schema_version":1,"id":"bbbb","name":"x"}"#)
            .unwrap();
        let report = run(&bucket, &pack_everything(), in_two_minutes()).unwrap();

        assert_eq!(report.removals.answered[0].packs_rewritten, [pack_key.as_str()]);
        let pack = bucket.get(&pack_key).unwrap().unwrap();
        let reader = PackReader::open(&pack).unwrap();
        let names: Vec<&str> = reader.entries().iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, [name("aaaa"), name("cccc")]);
        let mut keys: Vec<String> = bucket.list("").unwrap().into_iter().map(|object| object.key).collect();
        keys.sort();
        assert_eq!(
            keys,
            [pack_key.as_str(), "removals/done/bbbb", "removals/requested/bbbb", "removals/started/bbbb"]
        );
        keys.iter().for_each(|key| bucket.delete(key).unwrap());
    }

    /// Real submissions through the whole run: set TRACE_COMPACTOR_TEST_SUBMISSIONS to a month
    /// of a trace-data checkout, such as `.../trace-data/submissions/2026-11`. Every record of
    /// every pack is then compared with its file by the pack format's own check.
    #[test]
    #[ignore = "needs an S3 server and a trace-data checkout"]
    fn s3_run_packs_real_submissions_that_read_back_as_their_files() {
        let bucket = disposable("real");
        let month = std::path::PathBuf::from(std::env::var("TRACE_COMPACTOR_TEST_SUBMISSIONS").expect("set TRACE_COMPACTOR_TEST_SUBMISSIONS"));
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
            // Several packs, so the run goes round more than once, and a budget that makes
            // the builds wait for one another.
            pack_input_bytes: loose_bytes / 3,
            build_input_bytes: 2 << 20,
            ..pack_everything()
        };

        let packed = run(&bucket, &config, in_two_minutes()).unwrap().packed;

        assert!(packed.packs.len() >= 3 && packed.left_loose.is_empty(), "{packed:?}");
        let mut scratch = trace_pack::pack::Scratch::new();
        let mut found = Vec::new();
        for report in &packed.packs {
            assert_eq!(report.value_fallbacks, [] as [String; 0]);
            let pack = bucket.get(&report.key).unwrap().unwrap();
            let reader = PackReader::open(&pack).unwrap();
            for (index, entry) in reader.entries().iter().enumerate() {
                let file = std::fs::read(month.join(entry.name.strip_prefix(&format!("{folder}/")).unwrap())).unwrap();
                trace_pack::pack::check_against_file(&reader.record(index).unwrap(), entry, &file, &config.options.tp, &mut scratch)
                    .unwrap_or_else(|message| panic!("{}: {message}", entry.name));
                found.push(entry.name.clone());
            }
            bucket.delete(&report.key).unwrap();
        }
        found.sort();
        assert_eq!(found, names.iter().map(|name| format!("{folder}/{name}")).collect::<Vec<_>>());
        assert_eq!(bucket.list("").unwrap(), []);
        let pack_bytes: u64 = packed.packs.iter().map(|report| report.pack_bytes).sum();
        eprintln!(
            "{} real submissions: {loose_bytes} bytes of JSON in {} packs of {pack_bytes} bytes",
            names.len(),
            packed.packs.len()
        );
    }
}
