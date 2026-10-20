//! Implementasi Audio Processing Unit (APU) Game Boy Advance
//!
//! GBA memiliki sistem audio hybrid:
//! 1. 4 Channel Game Boy Legacy (PSG):
//!    - Channel 1: Square Wave dengan Frequency Sweep & Envelope
//!    - Channel 2: Square Wave dengan Envelope
//!    - Channel 3: Programmable Waveform (32 sampel 4-bit pada Wave RAM)
//!    - Channel 4: White Noise (LFSR pseudo-random)
//! 2. 2 Channel Direct Sound (DMA FIFO A & B):
//!    - Audio PCM 8-bit stereo berkualitas tinggi
//!    - Diberi makan secara otomatis oleh DMA 1 & 2 via pemicu Special
//!    - Di-clock oleh Timer 0 atau Timer 1

pub const SAMPLE_RATE: usize = 44_100;
pub const CPU_CLOCK_HZ: usize = 16_777_216; // 16.78 MHz
pub const CYCLES_PER_SAMPLE: usize = CPU_CLOCK_HZ / SAMPLE_RATE; // ~380.4 siklus per sampel audio

/// Antrean FIFO 32-byte untuk Direct Sound A / B
#[derive(Clone, Debug)]
pub struct SoundFifo {
    pub buffer: [i8; 32],
    pub read_ptr: usize,
    pub write_ptr: usize,
    pub count: usize,
    pub current_sample: i8,
}

impl Default for SoundFifo {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundFifo {
    pub fn new() -> Self {
        Self {
            buffer: [0; 32],
            read_ptr: 0,
            write_ptr: 0,
            count: 0,
            current_sample: 0,
        }
    }

    pub fn reset(&mut self) {
        self.read_ptr = 0;
        self.write_ptr = 0;
        self.count = 0;
        self.current_sample = 0;
    }

    /// Tulis 4 byte ke dalam FIFO (dari instruksi STR CPU atau transfer 32-bit DMA)
    pub fn push_word(&mut self, val: u32) {
        let bytes = val.to_le_bytes();
        for &b in &bytes {
            if self.count < 32 {
                self.buffer[self.write_ptr] = b as i8;
                self.write_ptr = (self.write_ptr + 1) % 32;
                self.count += 1;
            }
        }
    }

    /// Ambil satu sampel 8-bit saat di-clock oleh Timer overflow
    pub fn pop_sample(&mut self) -> i8 {
        if self.count > 0 {
            self.current_sample = self.buffer[self.read_ptr];
            self.read_ptr = (self.read_ptr + 1) % 32;
            self.count -= 1;
        }
        self.current_sample
    }
}

/// Channel 1: Square Wave dengan Sweep
#[derive(Clone, Debug, Default)]
pub struct Channel1 {
    pub sweep: u16,
    pub cnt_h: u16,
    pub cnt_x: u16,
    pub enabled: bool,
    pub duty: usize,
    pub period: usize,
    pub timer: usize,
    pub step: usize,
    pub volume: u8,
}

impl Channel1 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn write_sweep(&mut self, val: u16) {
        self.sweep = val;
    }

    pub fn write_cnt_h(&mut self, val: u16) {
        self.cnt_h = val;
        self.duty = ((val >> 6) & 0x03) as usize;
        self.volume = ((val >> 12) & 0x0F) as u8;
    }

    pub fn write_cnt_x(&mut self, val: u16) {
        self.cnt_x = val;
        let freq_raw = (val & 0x07FF) as usize;
        self.period = (2048 - freq_raw) * 4;
        if (val & (1 << 15)) != 0 {
            // Restart channel
            self.enabled = true;
            self.timer = self.period;
            self.step = 0;
        }
    }

    pub fn step(&mut self, cycles: usize) {
        if !self.enabled || self.period == 0 {
            return;
        }
        if cycles >= self.timer {
            self.timer = self.period;
            self.step = (self.step + 1) % 8;
        } else {
            self.timer -= cycles;
        }
    }

    pub fn sample(&self) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        // Pola duty cycle (12.5%, 25%, 50%, 75%)
        let duty_table: [[u8; 8]; 4] = [
            [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
            [1, 0, 0, 0, 0, 0, 0, 1], // 25%
            [1, 0, 0, 0, 0, 1, 1, 1], // 50%
            [0, 1, 1, 1, 1, 1, 1, 0], // 75%
        ];
        let wave = duty_table[self.duty][self.step];
        if wave == 1 {
            (self.volume as f32) / 15.0
        } else {
            -(self.volume as f32) / 15.0
        }
    }
}

/// Unit Pemrosesan Suara GBA
pub struct Apu {
    pub fifo_a: SoundFifo,
    pub fifo_b: SoundFifo,
    pub ch1: Channel1,

    // Register kontrol suara
    pub soundcnt_l: u16, // Master PSG volume & panning
    pub soundcnt_h: u16, // Direct Sound control & timer selection
    pub soundcnt_x: u16, // Sound Master Enable
    pub soundbias: u16,  // Sound Bias

    pub cycle_accumulator: usize,

    /// Buffer output stereo untuk konsumsi audio frontend
    pub output_samples: Vec<(f32, f32)>,
}

impl Default for Apu {
    fn default() -> Self {
        Self::new()
    }
}

impl Apu {
    pub fn new() -> Self {
        Self {
            fifo_a: SoundFifo::new(),
            fifo_b: SoundFifo::new(),
            ch1: Channel1::new(),
            soundcnt_l: 0,
            soundcnt_h: 0,
            soundcnt_x: 0x80, // Bit 7: Master Enable
            soundbias: 0x0200,
            cycle_accumulator: 0,
            output_samples: Vec::with_capacity(2048),
        }
    }

    pub fn reset(&mut self) {
        self.fifo_a.reset();
        self.fifo_b.reset();
        self.ch1.reset();
        self.soundcnt_l = 0;
        self.soundcnt_h = 0;
        self.soundcnt_x = 0x80;
        self.soundbias = 0x0200;
        self.cycle_accumulator = 0;
        self.output_samples.clear();
    }

    /// Cek apakah Timer yang dipilih untuk FIFO A / B overflow,
    /// dan kembalikan bitmask channel DMA yang perlu diisi ulang:
    /// Bit 0: DMA1 (FIFO A butuh data, count <= 16)
    /// Bit 1: DMA2 (FIFO B butuh data, count <= 16)
    pub fn on_timer_overflow(&mut self, timer_id: usize) -> u8 {
        let mut dma_request = 0;

        let dsa_enable = (self.soundcnt_h & (3 << 8)) != 0;
        let dsb_enable = (self.soundcnt_h & (3 << 12)) != 0;

        // Timer untuk Direct Sound A dipilih oleh bit 10 SOUNDCNT_H (0 = Timer 0, 1 = Timer 1)
        let timer_a = ((self.soundcnt_h >> 10) & 1) as usize;
        if dsa_enable && timer_id == timer_a {
            self.fifo_a.pop_sample();
            if self.fifo_a.count <= 16 {
                dma_request |= 1 << 0; // DMA1 request
            }
        }

        // Timer untuk Direct Sound B dipilih oleh bit 14 SOUNDCNT_H (0 = Timer 0, 1 = Timer 1)
        let timer_b = ((self.soundcnt_h >> 14) & 1) as usize;
        if dsb_enable && timer_id == timer_b {
            self.fifo_b.pop_sample();
            if self.fifo_b.count <= 16 {
                dma_request |= 1 << 1; // DMA2 request
            }
        }

        dma_request
    }

    /// Tulis register SOUNDCNT_H
    pub fn write_soundcnt_h(&mut self, val: u16) {
        self.soundcnt_h = val;
        // Bit 11: Reset FIFO A
        if (val & (1 << 11)) != 0 {
            self.fifo_a.reset();
        }
        // Bit 15: Reset FIFO B
        if (val & (1 << 15)) != 0 {
            self.fifo_b.reset();
        }
    }

    /// Majukan siklus waktu APU dan sintesis sampel audio jika interval tercapai
    pub fn step(&mut self, cycles: usize) {
        // Jika Master Sound dinonaktifkan (bit 7 SOUNDCNT_X), lewati sintesis
        if (self.soundcnt_x & (1 << 7)) == 0 {
            return;
        }

        self.ch1.step(cycles);

        self.cycle_accumulator += cycles;
        while self.cycle_accumulator >= CYCLES_PER_SAMPLE {
            self.cycle_accumulator -= CYCLES_PER_SAMPLE;
            self.generate_sample();
        }
    }

    /// Buat satu sampel audio stereo (Left, Right)
    fn generate_sample(&mut self) {
        // Direct Sound A & B volume (bit 2 & 3 SOUNDCNT_H: 0 = 50%, 1 = 100%)
        let dsa_vol = if (self.soundcnt_h & (1 << 2)) != 0 { 1.0 } else { 0.5 };
        let dsb_vol = if (self.soundcnt_h & (1 << 3)) != 0 { 1.0 } else { 0.5 };

        // Konversi sampel i8 (-128..127) ke rentang -1.0..1.0
        let sample_a = (self.fifo_a.current_sample as f32 / 128.0) * dsa_vol;
        let sample_b = (self.fifo_b.current_sample as f32 / 128.0) * dsb_vol;

        // Panning Direct Sound:
        // Bit 8: DSA Right, Bit 9: DSA Left
        // Bit 12: DSB Right, Bit 13: DSB Left
        let mut left = 0.0f32;
        let mut right = 0.0f32;

        if (self.soundcnt_h & (1 << 9)) != 0 { left += sample_a; }
        if (self.soundcnt_h & (1 << 8)) != 0 { right += sample_a; }

        if (self.soundcnt_h & (1 << 13)) != 0 { left += sample_b; }
        if (self.soundcnt_h & (1 << 12)) != 0 { right += sample_b; }

        // Tambahkan channel PSG (Channel 1)
        let ch1_sample = self.ch1.sample() * 0.25;
        left += ch1_sample;
        right += ch1_sample;

        // Soft limiter / clamp
        left = left.clamp(-1.0, 1.0);
        right = right.clamp(-1.0, 1.0);

        if self.output_samples.len() < 4096 {
            self.output_samples.push((left, right));
        }
    }

    /// Ambil seluruh sampel audio yang telah disintesis dan kosongkan buffer
    pub fn read_samples(&mut self) -> Vec<(f32, f32)> {
        std::mem::take(&mut self.output_samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sound_fifo_push_and_pop() {
        let mut fifo = SoundFifo::new();
        // Tulis 1 word (4 byte berturut-turut: 0x10, 0x20, 0x30, 0x40)
        fifo.push_word(0x4030_2010);
        assert_eq!(fifo.count, 4);

        assert_eq!(fifo.pop_sample(), 0x10);
        assert_eq!(fifo.pop_sample(), 0x20);
        assert_eq!(fifo.pop_sample(), 0x30);
        assert_eq!(fifo.pop_sample(), 0x40);
        assert_eq!(fifo.count, 0);
    }

    #[test]
    fn test_sound_timer_overflow_triggers_dma_request() {
        let mut apu = Apu::new();
        // Setup SOUNDCNT_H: Direct Sound A Timer = 0 (bit 10 = 0), Enable Left (bit 9), Enable Right (bit 8) -> 0x0304
        apu.write_soundcnt_h(0x0304);

        // Isi FIFO dengan 17 sampel
        for i in 0..4 {
            apu.fifo_a.push_word(0x0101_0101 * (i + 1));
        }
        apu.fifo_a.push_word(0x0505_0505);
        assert_eq!(apu.fifo_a.count, 20);

        // Overflow Timer 1: Tidak mempengaruhi FIFO A yang menggunakan Timer 0
        let req = apu.on_timer_overflow(1);
        assert_eq!(req, 0);

        // 3 kali Timer 0 overflow: count jadi 17 -> belum butuh DMA (count > 16)
        for _ in 0..3 {
            let req = apu.on_timer_overflow(0);
            assert_eq!(req, 0);
        }
        assert_eq!(apu.fifo_a.count, 17);

        // 1 kali Timer 0 overflow lagi: count jadi 16!
        // Sesuai arsitektur GBA: saat count <= 16, DMA1 Special langsung dipicu!
        let req = apu.on_timer_overflow(0);
        assert_eq!(req, 1); // Bit 0 (DMA1 request) aktif!
    }

    #[test]
    fn test_channel1_square_wave_synthesis() {
        let mut ch1 = Channel1::new();
        ch1.write_cnt_h(0xF080); // Volume 15, Duty 50% (indeks 2)
        ch1.write_cnt_x(0x8700); // Start channel + period

        assert!(ch1.enabled);
        ch1.step(ch1.period + 1);
        let s = ch1.sample();
        assert!(s != 0.0);
    }
}
