use rdev::{Button, EventType, Key, listen, simulate};
use serial_test::serial;
use std::error::Error;
use std::iter::Iterator;
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

fn sim_then_listen(events: &mut dyn Iterator<Item = EventType>) -> Result<(), Box<dyn Error>> {
    // spawn new thread because listen blocks
    let (sender, recv) = channel();
    let _listener = thread::spawn(move || {
        listen(move |event| {
            let _ = sender.send(event);
        })
        .expect("Could not listen");
    });
    let second = Duration::from_millis(1000);
    thread::sleep(second);

    #[cfg(target_os = "linux")]
    {
        let wheel = |delta_x, delta_y| EventType::Wheel { delta_x, delta_y };
        // Invalid or empty requests must not leave a stray click in the queue.
        assert!(simulate(&wheel(1.0, f64::INFINITY)).is_err());
        assert!(simulate(&wheel(1025.0, 0.0)).is_err());
        simulate(&wheel(0.0, 0.0))?;
        simulate(&wheel(2.0, -1.0))?;
        simulate(&wheel(-0.25, 0.0))?;
        for expected in [
            wheel(1.0, 0.0),
            wheel(1.0, 0.0),
            wheel(0.0, -1.0),
            wheel(-1.0, 0.0),
        ] {
            assert_eq!(recv.recv_timeout(second)?.event_type, expected);
        }
    }
    for event in events {
        simulate(&event)?;
        let recieved_event = recv.recv_timeout(second).expect("No events to recieve");
        assert_eq!(recieved_event.event_type, event);
    }
    Ok(())
}

#[test]
#[serial]
fn test_listen_and_simulate() -> Result<(), Box<dyn Error>> {
    // wait for user input from keyboard to stop
    // (i.e. the return/enter keypress to run test command)
    thread::sleep(Duration::from_millis(50));

    let events = vec![
        //TODO: fix sending shift keypress events on linux
        //EventType::KeyPress(Key::ShiftLeft),
        EventType::KeyPress(Key::KeyS),
        EventType::KeyRelease(Key::KeyS),
        EventType::ButtonPress(Button::Right),
        EventType::ButtonRelease(Button::Right),
        EventType::Wheel {
            delta_x: 0.0,
            delta_y: 1.0,
        },
        EventType::Wheel {
            delta_x: 0.0,
            delta_y: -1.0,
        },
    ]
    .into_iter();
    let click_events = (0..480).map(|pixel| EventType::MouseMove {
        x: pixel as f64,
        y: pixel as f64,
    });
    let mut events = events.chain(click_events);
    sim_then_listen(&mut events)
}
