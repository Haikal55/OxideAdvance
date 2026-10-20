/// Interrupt Controller (IE, IF, IME)
#[derive(Default)]
pub struct InterruptController {
    pub ime: bool, // Interrupt Master Enable
    pub ie: u16,   // Interrupt Enable
    pub if_: u16,  // Interrupt Flags (Request)
}

impl InterruptController {
    pub fn new() -> Self {
        Self {
            ime: false,
            ie: 0,
            if_: 0,
        }
    }

    pub fn has_pending_irq(&self) -> bool {
        self.ime && ((self.ie & self.if_) != 0)
    }

    pub fn request_irq(&mut self, mask: u16) {
        self.if_ |= mask;
    }
}
