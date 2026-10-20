/// Tombol fisik Game Boy Advance
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GbaKey {
    A = 0,
    B = 1,
    Select = 2,
    Start = 3,
    Right = 4,
    Left = 5,
    Up = 6,
    Down = 7,
    R = 8,
    L = 9,
}

/// Keypad Input Controller (Register KEYINPUT pada 0x0400_0130)
/// Karakteristik Hardware GBA: Active LOW (0 = Ditekan / Pressed, 1 = Dilepas / Released)
#[derive(Clone, Debug)]
pub struct Keypad {
    pub keyinput: u16,
    pub keycnt: u16,
}

impl Default for Keypad {
    fn default() -> Self {
        Self::new()
    }
}

impl Keypad {
    pub fn new() -> Self {
        Self {
            keyinput: 0x03FF, // 10 bit terendah bernilai 1 (semua tombol dilepas)
            keycnt: 0,
        }
    }

    /// Set status tombol apakah sedang ditekan (pressed) atau dilepas (released)
    pub fn set_key_pressed(&mut self, key: GbaKey, pressed: bool) {
        let bit = 1 << (key as u16);
        if pressed {
            self.keyinput &= !bit; // Active LOW: 0 = ditekan
        } else {
            self.keyinput |= bit;  // 1 = dilepas
        }
    }

    /// Periksa apakah tombol tertentu sedang ditekan
    pub fn is_key_pressed(&self, key: GbaKey) -> bool {
        let bit = 1 << (key as u16);
        (self.keyinput & bit) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keypad_active_low() {
        let mut keypad = Keypad::new();
        assert_eq!(keypad.keyinput, 0x03FF);
        assert!(!keypad.is_key_pressed(GbaKey::A));

        // Tekan tombol A
        keypad.set_key_pressed(GbaKey::A, true);
        assert_eq!(keypad.keyinput & 1, 0); // Bit 0 bernilai 0 saat ditekan
        assert!(keypad.is_key_pressed(GbaKey::A));

        // Lepas tombol A
        keypad.set_key_pressed(GbaKey::A, false);
        assert_eq!(keypad.keyinput & 1, 1);
        assert!(!keypad.is_key_pressed(GbaKey::A));
    }
}
