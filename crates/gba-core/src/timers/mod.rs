//! Implementasi Programmable Timers Game Boy Advance (Timer 0-3)
//!
//! GBA memiliki 4 timer independen beresolusi 16-bit.
//! Masing-masing timer memiliki prescaler (1, 64, 256, 1024 siklus)
//! dan mendukung cascade mode (Timer n berdetak saat Timer n-1 overflow).
//! Overflow timer dapat memicu IRQ dan sinkronisasi audio DirectSound FIFO.

/// Event overflow timer untuk pemicuan interrupt dan DirectSound
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimerEvents {
    pub overflows: [bool; 4],
}

/// Satu unit Timer 16-bit
#[derive(Clone, Debug, Default)]
pub struct Timer {
    pub id: usize,
    pub reload: u16,
    pub counter: u16,
    pub cnt_h: u16,
    pub prescaler_cycles: usize,
    pub enabled: bool,
}

impl Timer {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            reload: 0,
            counter: 0,
            cnt_h: 0,
            prescaler_cycles: 0,
            enabled: false,
        }
    }

    pub fn reset(&mut self) {
        self.reload = 0;
        self.counter = 0;
        self.cnt_h = 0;
        self.prescaler_cycles = 0;
        self.enabled = false;
    }

    /// Jumlah siklus per tick berdasarkan prescaler (bits 0-1)
    #[inline(always)]
    pub fn prescaler_period(&self) -> usize {
        match self.cnt_h & 0x03 {
            0 => 1,
            1 => 64,
            2 => 256,
            _ => 1024,
        }
    }

    /// Apakah timer ini menggunakan cascade mode (Count-up timing dari timer sebelumnya)
    /// Timer 0 tidak mendukung cascade mode
    #[inline(always)]
    pub fn is_cascade(&self) -> bool {
        self.id > 0 && (self.cnt_h & (1 << 2)) != 0
    }

    #[inline(always)]
    pub fn irq_enable(&self) -> bool {
        (self.cnt_h & (1 << 6)) != 0
    }

    #[inline(always)]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Tulis TMnCNT_L (mengatur nilai reload)
    pub fn write_cnt_l(&mut self, val: u16) {
        self.reload = val;
    }

    /// Baca TMnCNT_L (mengembalikan nilai counter saat ini)
    pub fn read_cnt_l(&self) -> u16 {
        self.counter
    }

    /// Tulis TMnCNT_H (mengatur prescaler, cascade, IRQ, dan start/stop)
    pub fn write_cnt_h(&mut self, val: u16) {
        let old_enabled = self.enabled;
        self.cnt_h = val;
        let new_enabled = (val & (1 << 7)) != 0;

        if !old_enabled && new_enabled {
            // Saat diaktifkan, counter dimuat ulang dari register reload
            self.counter = self.reload;
            self.prescaler_cycles = 0;
            self.enabled = true;
        } else if !new_enabled {
            self.enabled = false;
        }
    }

    /// Baca TMnCNT_H
    pub fn read_cnt_h(&self) -> u16 {
        (self.cnt_h & !0x0080) | ((self.enabled as u16) << 7)
    }
}

/// Controller untuk 4 unit Timer
pub struct Timers {
    pub timers: [Timer; 4],
}

impl Default for Timers {
    fn default() -> Self {
        Self::new()
    }
}

impl Timers {
    pub fn new() -> Self {
        Self {
            timers: [
                Timer::new(0),
                Timer::new(1),
                Timer::new(2),
                Timer::new(3),
            ],
        }
    }

    pub fn reset(&mut self) {
        for t in &mut self.timers {
            t.reset();
        }
    }

    /// Memajukan siklus waktu timer dan mengembalikan status overflow tiap timer
    pub fn step(&mut self, cycles: usize) -> TimerEvents {
        let mut events = TimerEvents::default();
        let mut prev_overflow = false;

        for id in 0..4 {
            let timer = &mut self.timers[id];
            if !timer.enabled {
                prev_overflow = false;
                continue;
            }

            let mut overflowed = false;

            if timer.is_cascade() {
                // Cascade mode: bertambah jika timer sebelumnya overflow
                if prev_overflow {
                    let next = timer.counter as u32 + 1;
                    if next > 0xFFFF {
                        timer.counter = timer.reload;
                        overflowed = true;
                    } else {
                        timer.counter = next as u16;
                    }
                }
            } else {
                // Prescaler mode
                let period = timer.prescaler_period();
                timer.prescaler_cycles += cycles;
                let ticks = timer.prescaler_cycles / period;
                timer.prescaler_cycles %= period;

                if ticks > 0 {
                    let next = timer.counter as u32 + ticks as u32;
                    if next > 0xFFFF {
                        let span = 0x10000 - timer.reload as u32;
                        if span > 0 {
                            let excess = next - 0x10000;
                            timer.counter = (timer.reload as u32 + (excess % span)) as u16;
                        } else {
                            timer.counter = timer.reload;
                        }
                        overflowed = true;
                    } else {
                        timer.counter = next as u16;
                    }
                }
            }

            events.overflows[id] = overflowed;
            prev_overflow = overflowed;
        }

        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timer_reload_and_start() {
        let mut timer = Timer::new(0);
        timer.write_cnt_l(0x1234);
        assert_eq!(timer.read_cnt_l(), 0); // Belum diaktifkan, counter masih 0

        timer.write_cnt_h(0x0080); // Start timer (bit 7 = 1, prescaler = 1 cycle)
        assert_eq!(timer.read_cnt_l(), 0x1234); // Counter dimuat dengan reload value
    }

    #[test]
    fn test_timer_overflow_and_prescaler() {
        let mut timers = Timers::new();
        // Setup Timer 0: reload = 0xFFFE, prescaler = /64 (0x0001), start (0x0080)
        timers.timers[0].write_cnt_l(0xFFFE);
        timers.timers[0].write_cnt_h(0x0081); // Start + Prescaler /64

        // 63 siklus: belum ada tick
        let ev = timers.step(63);
        assert!(!ev.overflows[0]);
        assert_eq!(timers.timers[0].read_cnt_l(), 0xFFFE);

        // +1 siklus (total 64 siklus): 1 tick -> counter jadi 0xFFFF
        let ev = timers.step(1);
        assert!(!ev.overflows[0]);
        assert_eq!(timers.timers[0].read_cnt_l(), 0xFFFF);

        // +64 siklus: 1 tick -> overflow ke reload (0xFFFE)
        let ev = timers.step(64);
        assert!(ev.overflows[0]);
        assert_eq!(timers.timers[0].read_cnt_l(), 0xFFFE);
    }

    #[test]
    fn test_timer_cascade_mode() {
        let mut timers = Timers::new();
        // Timer 0: reload 0xFFFF, prescaler /1, start
        timers.timers[0].write_cnt_l(0xFFFF);
        timers.timers[0].write_cnt_h(0x0080);

        // Timer 1: reload 0x0000, cascade mode (bit 2 = 1), start (bit 7 = 1)
        timers.timers[1].write_cnt_l(0x0000);
        timers.timers[1].write_cnt_h(0x0084);

        assert_eq!(timers.timers[1].read_cnt_l(), 0x0000);

        // Step 1 siklus: Timer 0 overflow -> Timer 1 bertambah 1
        let ev = timers.step(1);
        assert!(ev.overflows[0]);
        assert!(!ev.overflows[1]);
        assert_eq!(timers.timers[1].read_cnt_l(), 0x0001);
    }
}
