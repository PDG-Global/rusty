// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Parse raw terminal bytes into KeyEvent structs.
//!
//! Covers standard ANSI/VT100 escape sequences, CSI modifier encoding,
//! and the Kitty keyboard protocol (CSI u).

use crate::types::{KeyEvent, Modifier};

/// Parse a slice of raw bytes (as read from stdin in raw mode) into a KeyEvent.
///
/// Returns `None` if the bytes do not map to a recognised key.
pub fn parse_key_bytes(bytes: &[u8]) -> Option<KeyEvent> {
    if bytes.is_empty() {
        return None;
    }

    match bytes {
        // ── Single printable ASCII ────────────────────────────────
        [b @ 0x20..=0x7E] => Some(KeyEvent::plain((*b as char).to_string())),

        // ── Tab / Enter / Backspace / Escape ────────────────────
        [0x09] => Some(KeyEvent::plain("Tab")),
        [0x0D] | [0x0A] => Some(KeyEvent::plain("Enter")),
        [0x7F] => Some(KeyEvent::plain("Backspace")),
        [0x1B] => Some(KeyEvent::plain("Escape")),

        // ── Ctrl + letter (0x01-0x1A = Ctrl+A through Ctrl+Z) ───
        [b @ 0x01..=0x1A] => {
            let ch = (b'a' + *b - 1) as char;
            Some(KeyEvent::new(ch.to_string(), vec![Modifier::Ctrl]))
        }

        // ── Shift+Tab via CSI ────────────────────────────────────
        [0x1B, 0x5B, 0x5A] => Some(KeyEvent::new("Tab", vec![Modifier::Shift])),

        // ── CSI sequences: ESC [ ... ────────────────────────────
        [0x1B, 0x5B, rest @ ..] => parse_csi(rest),

        // ── Alt + key: ESC followed by a printable char ─────────
        [0x1B, b] if *b >= 0x20 && *b <= 0x7E => {
            Some(KeyEvent::new((*b as char).to_string(), vec![Modifier::Alt]))
        }

        _ => None,
    }
}

/// Parse the payload after `ESC [`.
fn parse_csi(bytes: &[u8]) -> Option<KeyEvent> {
    if bytes.is_empty() {
        return None;
    }

    // Kitty keyboard protocol: ESC [ code ; modifiers u
    if bytes.last() == Some(&b'u') {
        let inner = &bytes[..bytes.len() - 1];
        let parts: Vec<&[u8]> = inner.split(|b| *b == b';').collect();
        if parts.len() >= 2 {
            let code: u32 = std::str::from_utf8(parts[0]).ok()?.parse().ok()?;
            let mod_bits: u32 = std::str::from_utf8(parts[1]).ok()?.parse().ok()?;
            let ch = char::from_u32(code)?;
            let modifiers = decode_csi_modifiers(mod_bits);
            return Some(KeyEvent::new(ch.to_string(), modifiers));
        }
    }

    // Simple single-byte CSI finals (no parameters)
    match bytes {
        [b'A'] => return Some(KeyEvent::plain("Up")),
        [b'B'] => return Some(KeyEvent::plain("Down")),
        [b'C'] => return Some(KeyEvent::plain("Right")),
        [b'D'] => return Some(KeyEvent::plain("Left")),
        [b'H'] => return Some(KeyEvent::plain("Home")),
        [b'F'] => return Some(KeyEvent::plain("End")),
        [b'Z'] => return Some(KeyEvent::new("Tab", vec![Modifier::Shift])),
        _ => {}
    }

    // Delete: ESC [ 3 ~
    if bytes == [b'3', b'~'] {
        return Some(KeyEvent::plain("Delete"));
    }

    // Function keys: ESC [ 1 1 ~ through ESC [ 2 6 ~
    if bytes.len() >= 2 && bytes.last() == Some(&b'~') {
        let num_str = std::str::from_utf8(&bytes[..bytes.len() - 1]).ok()?;
        if let Ok(num) = num_str.parse::<u8>() {
            let name = match num {
                11 => "F1",
                12 => "F2",
                13 => "F3",
                14 => "F4",
                15 => "F5",
                17 => "F6",
                18 => "F7",
                19 => "F8",
                20 => "F9",
                21 => "F10",
                23 => "F11",
                24 => "F12",
                _ => return None,
            };
            return Some(KeyEvent::plain(name));
        }
    }

    // CSI with modifiers: ESC [ params ; modifier <final>
    // e.g. ESC [ 1 ; 5 A = Ctrl+Up
    if let Some(semi_pos) = bytes.iter().position(|&b| b == b';') {
        let final_byte = bytes.last().copied()?;
        let mod_str = std::str::from_utf8(&bytes[semi_pos + 1..bytes.len() - 1]).ok()?;
        let mod_bits: u32 = mod_str.parse().ok()?;
        let key = match final_byte {
            b'A' => "Up",
            b'B' => "Down",
            b'C' => "Right",
            b'D' => "Left",
            b'H' => "Home",
            b'F' => "End",
            b'~' => {
                // ESC [ params ; modifier ~ style sequences (e.g. Delete with mods)
                let param_str = std::str::from_utf8(&bytes[..semi_pos]).ok()?;
                match param_str {
                    "3" => "Delete",
                    _ => return None,
                }
            }
            _ => return None,
        };
        let modifiers = decode_csi_modifiers(mod_bits);
        return Some(KeyEvent::new(key, modifiers));
    }

    None
}

/// Decode CSI modifier bitmask. The wire value is `mask + 1` where:
///   bit 0 = Shift, bit 1 = Alt, bit 2 = Ctrl, bit 3 = Super.
fn decode_csi_modifiers(bits: u32) -> Vec<Modifier> {
    let mask = bits.saturating_sub(1);
    let mut mods = Vec::new();
    if mask & 1 != 0 {
        mods.push(Modifier::Shift);
    }
    if mask & 2 != 0 {
        mods.push(Modifier::Alt);
    }
    if mask & 4 != 0 {
        mods.push(Modifier::Ctrl);
    }
    if mask & 8 != 0 {
        mods.push(Modifier::Super);
    }
    mods
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ascii() {
        let ev = parse_key_bytes(b"q").unwrap();
        assert_eq!(ev, KeyEvent::plain("q"));
    }

    #[test]
    fn ctrl_c() {
        let ev = parse_key_bytes(&[0x03]).unwrap();
        assert_eq!(ev, KeyEvent::new("c", vec![Modifier::Ctrl]));
    }

    #[test]
    fn escape() {
        let ev = parse_key_bytes(&[0x1B]).unwrap();
        assert_eq!(ev, KeyEvent::plain("Escape"));
    }

    #[test]
    fn arrow_up() {
        let ev = parse_key_bytes(b"\x1b[A").unwrap();
        assert_eq!(ev, KeyEvent::plain("Up"));
    }

    #[test]
    fn ctrl_up_csi_modifier() {
        // ESC [ 1 ; 5 A
        let ev = parse_key_bytes(b"\x1b[1;5A").unwrap();
        assert_eq!(ev, KeyEvent::new("Up", vec![Modifier::Ctrl]));
    }

    #[test]
    fn shift_tab() {
        let ev = parse_key_bytes(b"\x1b[Z").unwrap();
        assert_eq!(ev, KeyEvent::new("Tab", vec![Modifier::Shift]));
    }

    #[test]
    fn delete() {
        let ev = parse_key_bytes(b"\x1b[3~").unwrap();
        assert_eq!(ev, KeyEvent::plain("Delete"));
    }

    #[test]
    fn alt_key() {
        // ESC then 'x'
        let ev = parse_key_bytes(b"\x1bx").unwrap();
        assert_eq!(ev, KeyEvent::new("x", vec![Modifier::Alt]));
    }

    #[test]
    fn kitty_ctrl_c() {
        // ESC [ 99 ; 5 u = Ctrl+C (kitty protocol)
        let ev = parse_key_bytes(b"\x1b[99;5u").unwrap();
        assert_eq!(ev, KeyEvent::new("c", vec![Modifier::Ctrl]));
    }

    #[test]
    fn function_key_f1() {
        let ev = parse_key_bytes(b"\x1b[11~").unwrap();
        assert_eq!(ev, KeyEvent::plain("F1"));
    }

    #[test]
    fn enter() {
        let ev = parse_key_bytes(b"\r").unwrap();
        assert_eq!(ev, KeyEvent::plain("Enter"));
    }

    #[test]
    fn shift_up_csi_modifier() {
        // ESC [ 1 ; 2 A = Shift+Up
        let ev = parse_key_bytes(b"\x1b[1;2A").unwrap();
        assert_eq!(ev, KeyEvent::new("Up", vec![Modifier::Shift]));
    }

    #[test]
    fn alt_right_csi_modifier() {
        // ESC [ 1 ; 3 C = Alt+Right
        let ev = parse_key_bytes(b"\x1b[1;3C").unwrap();
        assert_eq!(ev, KeyEvent::new("Right", vec![Modifier::Alt]));
    }

    #[test]
    fn ctrl_shift_left_csi_modifier() {
        // ESC [ 1 ; 6 D = Ctrl+Shift+Left
        let ev = parse_key_bytes(b"\x1b[1;6D").unwrap();
        assert_eq!(ev, KeyEvent::new("Left", vec![Modifier::Shift, Modifier::Ctrl]));
    }

    #[test]
    fn delete_with_modifier() {
        // ESC [ 3 ; 5 ~ = Ctrl+Delete
        let ev = parse_key_bytes(b"\x1b[3;5~").unwrap();
        assert_eq!(ev, KeyEvent::new("Delete", vec![Modifier::Ctrl]));
    }

    #[test]
    fn backspace() {
        let ev = parse_key_bytes(&[0x7F]).unwrap();
        assert_eq!(ev, KeyEvent::plain("Backspace"));
    }
}