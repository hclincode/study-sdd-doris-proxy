//! Task 2.4 — invalid configuration prevents startup **before any listener
//! binds**.
//!
//! `policy-config` owns the diagnostics; this file owns the ordering, which is
//! the part that is a security property rather than a usability one. A proxy
//! that binds first and validates second is reachable, however briefly, while
//! enforcing nothing.
//!
//! These tests run the real binary as a subprocess. Asserting on the exit status
//! alone would not be enough — a process can fail *after* having bound a socket —
//! so each negative case also checks that nothing ever accepted a connection on
//! the listen address.

use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BINARY: &str = env!("CARGO_BIN_EXE_doris-row-filter-proxy");

/// An address nothing is listening on, obtained by binding and releasing.
fn free_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

fn write_temp(name: &str, contents: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "doris-proxy-{}-{}-{name}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(contents.as_bytes()).unwrap();
    path
}

/// Whether anything accepts a connection on `addr` within `window`.
fn anything_listening(addr: SocketAddr, window: Duration) -> bool {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(50)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn run_proxy(policy: &PathBuf, listen: SocketAddr, backend: SocketAddr) -> std::process::Child {
    Command::new(BINARY)
        .arg("--policy")
        .arg(policy)
        .arg("--listen")
        .arg(listen.to_string())
        .arg("--backend")
        .arg(backend.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to launch the proxy binary")
}

#[test]
fn unparseable_configuration_prevents_the_listener_from_binding() {
    let policy = write_temp("bad.toml", "this is not [ valid toml {{{\n");
    let listen = free_address();
    let backend = free_address();

    let output = run_proxy(&policy, listen, backend)
        .wait_with_output()
        .unwrap();

    assert!(
        !output.status.success(),
        "the proxy started despite unparseable configuration"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("refusing to start"),
        "stderr did not explain the refusal: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !anything_listening(listen, Duration::from_millis(200)),
        "something bound the listen address despite invalid configuration"
    );

    let _ = std::fs::remove_file(&policy);
}

#[test]
fn a_misspelled_policy_section_prevents_startup_rather_than_disabling_filtering() {
    // The failure mode this guards: a file that parses cleanly but yields zero
    // policies would start a proxy that filters nothing at all.
    let policy = write_temp(
        "misspelled.toml",
        "[[policies]]\n\
         user = \"analyst\"\n\
         database = \"sales\"\n\
         table = \"orders\"\n\
         column = \"region\"\n\
         permitted_values = [\"APAC\"]\n",
    );
    let listen = free_address();
    let backend = free_address();

    let output = run_proxy(&policy, listen, backend)
        .wait_with_output()
        .unwrap();

    assert!(
        !output.status.success(),
        "a misspelled section started a proxy with zero policies"
    );
    assert!(
        !anything_listening(listen, Duration::from_millis(200)),
        "something bound the listen address despite invalid configuration"
    );

    let _ = std::fs::remove_file(&policy);
}

/// The control for the two tests above: with a *valid* file the listener does
/// bind, so their assertions are about the configuration and not about the
/// binary failing to start for some unrelated reason.
#[test]
fn valid_configuration_does_bind_the_listener() {
    let policy = write_temp(
        "good.toml",
        "[[policy]]\n\
         user = \"analyst\"\n\
         database = \"sales\"\n\
         table = \"orders\"\n\
         column = \"region\"\n\
         permitted_values = [\"APAC\", \"EMEA\"]\n",
    );
    let listen = free_address();
    // Deliberately unreachable: an unavailable frontend must not stop the proxy
    // from starting. Sessions are refused one by one instead (task 3.5).
    let backend = free_address();

    let mut child = run_proxy(&policy, listen, backend);
    let bound = anything_listening(listen, Duration::from_secs(5));
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&policy);

    assert!(
        bound,
        "the proxy did not bind its listener with a valid policy file"
    );
}
