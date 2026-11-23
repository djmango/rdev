use rdev::Key as RdevKey;

#[test]
fn test_convet_keycode() {
    let key = RdevKey::KeyQ;
    let (keycode, scancode) = (81, 16);

    assert_eq!(key, rdev::get_win_key(keycode, scancode));
    assert_eq!((81, 16), rdev::get_win_codes(key).unwrap());

    assert_eq!(16, rdev::win_scancode_from_key(key).unwrap()); // Windows
    assert_eq!(24, rdev::linux_keycode_from_key(key).unwrap()); // Linux
    assert_eq!(12, rdev::macos_keycode_from_key(key).unwrap()); // Mac OS
}

#[test]
fn korean_x11_keycodes() {
    for (key, code) in [(RdevKey::Hangul, 0x82), (RdevKey::Hanja, 0x83)] {
        assert_eq!(rdev::linux_keycode_from_key(key), Some(code));
        assert_eq!(rdev::linux_key_from_code(code), key);
    }
}

#[test]
fn buttons_as_hash_keys() {
    use rdev::Button;
    let buttons = [
        Button::Left,
        Button::Unknown(1),
        Button::Unknown(2),
        Button::Unknown(1),
    ];
    let set: std::collections::HashSet<_> = buttons.into_iter().collect();
    assert_eq!(set.len(), 3);
    assert!(set.contains(&Button::Unknown(2)));
}
