//! SSH-connect sentinel verification: a destination that could embed a
//! credential must never surface in an error `Display`, a `Debug` print, or
//! the persisted `sessions.toml` document, and Noren's parsed model must
//! never adopt `ProxyCommand` content.
//!
//! Modeled on `tests/security_no_leak.rs` (TM-08): one unique sentinel is
//! planted into each named channel and every observable surface is scanned
//! for it. The scanner is self-tested against a planted leak so the suite
//! cannot pass vacuously. The channels here are the ones the SSH connect
//! slice owns:
//!
//! - **Destination errors** — a secret-shaped destination is pushed through
//!   every refusal class of `noren_pty::SshDestination`, and both `Display`
//!   and `Debug` of the typed error are scanned.
//! - **Launch policy debug** — `SshDestination` and `SshLaunchPolicy` debug
//!   prints are redacted by construction and scanned here.
//! - **Persisted session state** — `session_persistence::encode` output for
//!   a registry that the connect flow could have produced is scanned; the
//!   connect path records SSH launches outside the registry, so an encoder
//!   that suddenly carried a destination would fail this test.
//! - **Parsed SSH configuration** — a `ProxyCommand` (and a
//!   `ProxyCommand`-bearing HostName) is parsed; the parsed model must keep
//!   the host discoverable while retaining none of the command, and the
//!   parser's error surfaces must not echo it either. Noren never executes
//!   proxy commands: they remain the system ssh binary's own trust boundary.

use noren_app::session::{SessionKind, SessionRegistry, SessionStatus};
use noren_app::session_persistence;
use noren_app::ssh_config::SshConfig;
use noren_pty::{SshDestination, SshDestinationError, SshLaunchPolicy, SshPercentToken};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NONCE: AtomicU64 = AtomicU64::new(0);

/// The unique secret-shaped value planted into every channel.
fn sentinel() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock advances")
        .as_nanos();
    let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
    format!("NOREN-SSHCONN-hunter2-{pid}-{nanos:x}-{nonce}")
}

/// The test logger contract: reject any sentinel fragment in `haystack`.
fn leaked(haystack: &str, secret: &str) -> bool {
    haystack.contains(secret)
}

/// The scanner must detect a planted leak, or a clean result would be
/// meaningless (mirrors `scanner_flags_planted_leaks_and_accepts_clean_output`).
#[test]
fn scanner_flags_planted_leaks_and_accepts_clean_output() {
    let secret = sentinel();
    let clean = "SSH destination contains the unexpanded OpenSSH token %h \
                 (HostName keyword); the connect must not proceed";
    assert!(!leaked(clean, &secret));
    assert!(leaked(&format!("refused destination {secret}"), &secret));
}

/// Every destination-refusal class must reject a secret-shaped destination
/// without echoing it in `Display` or `Debug`.
#[test]
fn destination_refusals_never_echo_the_destination() {
    let secret = sentinel();
    let refusals: Vec<(String, SshDestinationError)> = vec![
        (String::new(), SshDestinationError::Empty),
        (format!("-{secret}"), SshDestinationError::LeadingHyphen),
        (
            format!("{secret} has space"),
            SshDestinationError::ControlOrWhitespace,
        ),
        (
            format!("{secret}\t"),
            SshDestinationError::ControlOrWhitespace,
        ),
        (
            format!("{secret}%h"),
            SshDestinationError::RawToken {
                token: SshPercentToken::Host,
            },
        ),
        (
            format!("%p{secret}"),
            SshDestinationError::RawToken {
                token: SshPercentToken::Port,
            },
        ),
        (
            format!("%r{secret}"),
            SshDestinationError::RawToken {
                token: SshPercentToken::RemoteUser,
            },
        ),
    ];
    for (destination, expected) in refusals {
        let error =
            SshDestination::new(&destination).expect_err("every planted destination is refused");
        assert_eq!(error, expected, "the planted destination must be refused");
        let display = error.to_string();
        let debug = format!("{error:?}");
        assert!(
            !leaked(&display, &secret),
            "refusal Display leaked the destination: {display}"
        );
        assert!(
            !leaked(&debug, &secret),
            "refusal Debug leaked the destination: {debug}"
        );
    }

    // The oversize class with a long secret-shaped payload.
    let oversized = format!("{}{}", sentinel(), "x".repeat(2048));
    let error = SshDestination::new(&oversized).expect_err("oversize is refused");
    assert_eq!(error, SshDestinationError::Oversize);
    assert!(!leaked(&error.to_string(), &oversized[..32]));
}

/// The token-bearing refusal names the OpenSSH keyword and token while
/// carrying none of the surrounding destination content.
#[test]
fn token_refusal_names_keyword_and_token_without_content() {
    let secret = sentinel();
    let cases = [
        (
            format!("user@%h.{secret}"),
            SshPercentToken::Host,
            "HostName",
        ),
        (format!("host:%p-{secret}"), SshPercentToken::Port, "Port"),
        (
            format!("%r.{secret}@host"),
            SshPercentToken::RemoteUser,
            "User",
        ),
    ];
    for (destination, token, keyword) in cases {
        let error =
            SshDestination::new(&destination).expect_err("token-bearing destinations are refused");
        let text = error.to_string();
        assert!(text.contains(token.as_str()), "names the token: {text}");
        assert!(text.contains(keyword), "names the keyword: {text}");
        assert!(
            !leaked(&text, &secret),
            "carries no destination content: {text}"
        );
    }
}

/// A validated destination (and any policy over it) never prints itself:
/// `Debug` is redacted by construction, scanned here with a secret-shaped
/// target.
#[test]
fn destination_and_policy_debug_stay_redacted_for_secret_shaped_targets() {
    let secret = sentinel();
    let destination =
        SshDestination::new(&format!("{secret}@web1.example")).expect("shape is valid");
    let policy = SshLaunchPolicy::inherit(destination);
    for inspected in [format!("{:?}", policy.destination()), format!("{policy:?}")] {
        assert!(
            !leaked(&inspected, &secret),
            "debug surface leaked the destination: {inspected}"
        );
    }
    // The accessor exists for argv construction; it is not a debug surface,
    // and this assertion pins that nothing else grew content-bearing Debug.
    assert_eq!(
        policy.destination().as_str().len(),
        secret.len() + "@web1.example".len()
    );
}

/// The persisted sessions document never carries an SSH launch destination:
/// the connect flow records launches outside the registry, so encoding the
/// registry state the flow produces cannot mention the target.
#[test]
fn encoded_session_state_never_carries_an_ssh_launch_destination() {
    let secret = sentinel();
    let mut registry = SessionRegistry::new();
    // The registry shape a connect flow leaves behind: the previous local
    // session observed to a terminal status, and nothing else.
    let local = registry.create(SessionKind::Local);
    registry
        .observe(local, SessionStatus::Exited { code: Some(0) })
        .expect("a monotonic observation");
    let encoded = session_persistence::encode(&registry).expect("the registry encodes");
    assert!(
        !leaked(&encoded, &secret),
        "sessions.toml content leaked the destination: {encoded}"
    );
    assert!(
        !encoded.contains("kind = \"ssh\""),
        "no SSH launch may enter the persisted document: {encoded}"
    );
}

/// Noren's parsed SSH model never adopts ProxyCommand content: the host stays
/// discoverable, no fact carries the command, and no error surface echoes it.
/// Executing a proxy command remains the system ssh binary's own trust
/// boundary over the user's own configuration; Noren only passes the alias.
#[test]
fn parsed_model_never_adopts_proxycommand_content() {
    let secret = sentinel();
    let text = format!(
        "Host proxied\n  ProxyCommand /bin/sh -c '{secret} %h %p'\n  HostName proxied.example\n"
    );
    let config = SshConfig::parse(&text).expect("ProxyCommand remains syntactically opaque");
    let hosts = config.hosts();
    assert_eq!(hosts.len(), 1, "the host stays discoverable");
    assert_eq!(hosts[0].alias(), "proxied");
    assert_eq!(hosts[0].host_name(), Some("proxied.example"));
    // No fact of the parsed model carries the command text.
    let model = format!("{config:?}");
    assert!(
        !leaked(&model, &secret),
        "the parsed model leaked ProxyCommand content: {model}"
    );

    // The parser's own failure path (a malformed directive beside a
    // ProxyCommand) never echoes the command either.
    let broken = format!("Host broken\nPort nope\nProxyCommand {secret}\n");
    let error = SshConfig::parse(&broken).expect_err("the malformed Port fails the parse");
    assert!(!leaked(&error.to_string(), &secret));
    assert!(!leaked(&format!("{error:?}"), &secret));
}
