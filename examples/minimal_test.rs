fn main() {
    println!("Minimal rdev test - press any key or move mouse");
    println!("You should see events below:");
    println!();

    if let Err(error) = rdev::listen(|event| {
        println!("Got event: {:?}", event.event_type);
    }) {
        eprintln!("Error: {:?}", error);
    }
}
