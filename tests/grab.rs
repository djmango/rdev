#![cfg(target_os = "windows")]
use rdev::{Event, EventType, Key, grab, listen, simulate};
use serial_test::serial;
use std::error::Error;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::Duration;

static EVENT_CHANNEL: LazyLock<(Mutex<Sender<Event>>, Mutex<Receiver<Event>>)> =
    LazyLock::new(|| {
        let (send, recv) = channel();
        (Mutex::new(send), Mutex::new(recv))
    });

fn send_event(event: Event) {
    EVENT_CHANNEL
        .0
        .lock()
        .expect("Failed to unlock Mutex")
        .send(event)
        .expect("Receiving end of EVENT_CHANNEL was closed");
}

fn grab_tab(event: Event) -> Option<Event> {
    match event.event_type {
        EventType::KeyPress(Key::Tab) => None,
        EventType::KeyRelease(Key::Tab) => None,
        _ => Some(event),
    }
}

#[test]
#[ignore = "requires an isolated Windows desktop and temporarily suppresses Tab"]
#[serial]
fn test_grab() -> Result<(), Box<dyn Error>> {
    // Wait for tester's key to go back up
    // otherwise, test fails due to KeyRelease(Return)
    thread::sleep(Duration::from_millis(300));

    // spawn new thread because listen blocks
    let _listener = thread::spawn(move || {
        listen(send_event).expect("Could not listen");
    });
    // Make sure grab ends up on top of listen so it can properly discard.
    thread::sleep(Duration::from_secs(1));
    let _grab = thread::spawn(move || {
        grab(grab_tab).expect("Could not grab");
    });

    let recv = EVENT_CHANNEL.1.lock().expect("Failed to unlock Mutex");

    // Wait for listen to start
    thread::sleep(Duration::from_secs(1));

    let event_type = EventType::KeyPress(Key::KeyS);
    let event_type2 = EventType::KeyRelease(Key::KeyS);
    simulate(&event_type)?;
    simulate(&event_type2)?;

    let timeout = Duration::from_secs(1);
    assert_eq!(event_type, recv.recv_timeout(timeout)?.event_type);
    assert_eq!(event_type2, recv.recv_timeout(timeout)?.event_type);

    simulate(&EventType::KeyPress(Key::Tab))?;
    simulate(&EventType::KeyRelease(Key::Tab))?;
    match recv.recv_timeout(timeout) {
        Ok(event) => panic!("We should not receive event : {:?}", event),
        Err(err) => assert_eq!(err, RecvTimeoutError::Timeout),
    };
    Ok(())
}
