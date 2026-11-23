use rdev::{EventType, listen};

fn main() {
    if let Err(error) = listen(|event| {
        if let EventType::ButtonPress(button) = event.event_type {
            println!("Mouse button pressed: {button:?}");
        }
    }) {
        eprintln!("Could not listen: {error:?}");
        std::process::exit(1);
    }
}
