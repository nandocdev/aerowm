//! Framing regression tests (Fase 2, punto 7): the IPC server must
//! reassemble frames regardless of how stream sockets split writes.

use aerowm_ipc::read_frame;
use std::io::Write;
use std::os::unix::net::UnixStream;

fn pair() -> (UnixStream, UnixStream) {
    UnixStream::pair().expect("socketpair")
}

#[test]
fn whole_single_write() {
    let (mut tx, mut rx) = pair();
    tx.write_all(b"{\"cmd\":1}\n").unwrap();
    drop(tx);
    assert_eq!(read_frame(&mut rx).unwrap(), Some("{\"cmd\":1}".into()));
}

#[test]
fn byte_at_a_time_fragmentation() {
    let (mut tx, mut rx) = pair();
    let payload = b"{\"Workspace\":{\"action\":\"Next\"}}\n";
    for chunk in payload.chunks(1) {
        tx.write_all(chunk).unwrap();
    }
    drop(tx);
    assert_eq!(
        read_frame(&mut rx).unwrap(),
        Some("{\"Workspace\":{\"action\":\"Next\"}}".into())
    );
}

#[test]
fn no_newline_ends_at_eof() {
    let (mut tx, mut rx) = pair();
    // aerowm-ctl always sends `\n`, but be lenient with what we got.
    tx.write_all(b"{\"cmd\":2}").unwrap();
    drop(tx);
    assert_eq!(read_frame(&mut rx).unwrap(), Some("{\"cmd\":2}".into()));
}

#[test]
fn coalesced_writes_yield_first_frame() {
    let (mut tx, mut rx) = pair();
    // One connection = one command; trailing bytes are not our protocol.
    tx.write_all(b"{\"a\":1}\n{\"b\":2}\n").unwrap();
    drop(tx);
    assert_eq!(read_frame(&mut rx).unwrap(), Some("{\"a\":1}".into()));
}

#[test]
fn empty_eof_is_none() {
    let (tx, mut rx) = pair();
    drop(tx);
    assert_eq!(read_frame(&mut rx).unwrap(), None);
}

#[test]
fn oversized_frame_rejected() {
    let (mut tx, mut rx) = pair();
    tx.write_all(&vec![b'x'; aerowm_ipc::MAX_FRAME_BYTES + 1024]).unwrap();
    // No newline and over the cap: must error, never buffer forever.
    assert!(read_frame(&mut rx).is_err());
}
