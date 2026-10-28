//! Exercise the production send path with real nonblocking Windows named pipes.
use super::*;
use crate::native_terminal::{Bounds, NativeTerminalManager};

fn pair() -> (Pipe, Pipe) {
    let endpoint = format!("kanban-pipe-test-{}", uuid::Uuid::new_v4());
    let server = Pipe::listen(&endpoint).unwrap();
    let client = Pipe::connect(Path::new(&endpoint)).unwrap();
    assert!(server.accept().unwrap());
    (server, client)
}

fn session() -> (NativeTerminalManager, Arc<Session>) {
    let manager = NativeTerminalManager::default();
    let view = manager
        .select(
            "pipe-test".into(),
            "task".into(),
            Bounds {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 100.,
                viewport_width: 100.,
                viewport_height: 100.,
                visible: true,
            },
            Arc::new(|_| {}),
        )
        .unwrap();
    (manager, Session::new(view, "test:p1".into()))
}

#[test]
fn large_frame_reaches_native_client_unchanged() {
    let (mut sender, mut receiver) = pair();
    let (_manager, session) = session();
    let payload = b"native frame: \0\xff\xe4\xb8\xad\xe6\x96\x87".repeat(4096);
    let expected = wire::frame(&payload);
    assert!(expected.len() > 65536);
    let size = expected.len();
    let reader = thread::spawn(move || {
        let mut received = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(6);
        while received.len() < size && Instant::now() < deadline {
            if !receiver.read_available(&mut received).unwrap() {
                thread::sleep(Duration::from_millis(1));
            }
        }
        received
    });
    let result = sender.send(&expected, &session, &AtomicBool::new(false));
    let mut received = reader.join().unwrap();
    assert_eq!(result, Ok(()));
    assert_eq!(received, expected);
    assert_eq!(wire::next_frame(&mut received).unwrap(), Some(payload));
    assert!(received.is_empty());
}

#[test]
fn framed_stream_preserves_boundaries_and_order_in_both_pipe_directions() {
    for reverse in [false, true] {
        let (server, client) = pair();
        let (mut sender, mut receiver) = if reverse {
            (client, server)
        } else {
            (server, client)
        };
        let (_manager, session) = session();
        // Sizes include the four-byte frame header.
        let payloads: Vec<Vec<u8>> = [17, 65535, 65536, 65537, 80 * 1024, 256 * 1024, 1024 * 1024]
            .into_iter()
            .map(|size| (0..size - 4).map(|i| (i % 256) as u8).collect())
            .collect();
        let frames: Vec<_> = payloads.iter().map(|p| wire::frame(p)).collect();
        let expected = frames.concat();
        let size = expected.len();
        let reader = thread::spawn(move || {
            // Initially leave the pipe full, then consume it in small, uneven reads.
            thread::sleep(Duration::from_millis(50));
            let deadline = Instant::now() + Duration::from_secs(6);
            let mut received = Vec::new();
            let mut buffer = [0; 4093];
            while received.len() < size && Instant::now() < deadline {
                match receiver.0.read(&mut buffer) {
                    Ok(0) => thread::sleep(Duration::from_millis(1)),
                    Ok(count) => received.extend_from_slice(&buffer[..count]),
                    Err(e) if e.raw_os_error() == Some(ERROR_NO_DATA as i32) => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(e) => panic!("pipe receiver failed: {e}"),
                }
            }
            received
        });
        let result: Result<(), _> = frames
            .iter()
            .try_for_each(|frame| sender.send(frame, &session, &AtomicBool::new(false)));
        let mut received = reader.join().unwrap();
        assert_eq!(result, Ok(()));
        assert_eq!(received, expected);
        for payload in payloads {
            assert_eq!(wire::next_frame(&mut received).unwrap(), Some(payload));
        }
        assert!(received.is_empty());
    }
}

#[test]
fn slow_progress_does_not_extend_the_whole_send_deadline() {
    let (mut sender, mut receiver) = pair();
    let (_manager, session) = session();
    let (done, finished) = std::sync::mpsc::channel();
    let reader = thread::spawn(move || {
        let mut count = 0;
        let mut buffer = [0; 4096];
        while matches!(
            finished.recv_timeout(Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ) {
            match receiver.0.read(&mut buffer) {
                Ok(n) => count += n,
                Err(e) if e.raw_os_error() == Some(ERROR_NO_DATA as i32) => {}
                Err(e) => panic!("pipe receiver failed: {e}"),
            }
        }
        count
    });
    let started = Instant::now();
    let result = sender.send(&vec![0x5a; 1024 * 1024], &session, &AtomicBool::new(false));
    let elapsed = started.elapsed();
    done.send(()).unwrap();
    let received = reader.join().unwrap();
    assert_eq!(result, Err("terminal.client_focus_timeout".into()));
    assert!(received > 65536 && received < 1024 * 1024);
    assert!(elapsed >= Duration::from_secs(5));
    assert!(elapsed < Duration::from_secs(7));
}

#[test]
fn unread_pipe_times_out_without_waiting_forever() {
    let (mut sender, _receiver) = pair();
    let (_manager, session) = session();
    let started = Instant::now();
    let result = sender.send(&vec![0x5a; 256 * 1024], &session, &AtomicBool::new(false));
    assert_eq!(result, Err("terminal.client_focus_timeout".into()));
    assert!(started.elapsed() < Duration::from_secs(7));
}

#[test]
fn cancellation_session_close_and_peer_exit_interrupt_a_blocked_send() {
    for ending in ["cancel", "session", "peer"] {
        let (mut sender, mut receiver) = pair();
        let (_manager, session) = session();
        let cancelled = Arc::new(AtomicBool::new(false));
        let sending_session = session.clone();
        let sending_cancelled = cancelled.clone();
        let writer = thread::spawn(move || {
            sender.send(
                &vec![0x5a; 1024 * 1024],
                &sending_session,
                &sending_cancelled,
            )
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut prefix = Vec::new();
        while !receiver.read_available(&mut prefix).unwrap() {
            assert!(
                Instant::now() < deadline,
                "send never reached the receiving pipe"
            );
            thread::sleep(Duration::from_millis(1));
        }
        let started = Instant::now();
        match ending {
            "cancel" => cancelled.store(true, Ordering::Release),
            "session" => session.stop("terminal.closed"),
            "peer" => drop(receiver),
            _ => unreachable!(),
        }
        let error = writer.join().unwrap().unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(if ending == "peer" { 7 } else { 2 }));
        match ending {
            "cancel" => assert_eq!(error, "terminal.stale"),
            "session" => assert_eq!(error, "terminal.disconnected"),
            // Windows may report ERROR_NO_DATA on peer exit, which the existing
            // send contract retries only until its whole-send deadline.
            "peer" => assert!(
                error.starts_with("terminal.client_connect:")
                    || error == "terminal.client_focus_timeout"
            ),
            _ => unreachable!(),
        }
    }
}
