//! Implementasi Direct Memory Access (DMA) Controller Game Boy Advance
//!
//! GBA memiliki 4 channel DMA independen (Channel 0-3) dengan prioritas:
//! Channel 0 (tertinggi) -> Channel 1 -> Channel 2 -> Channel 3 (terendah).
//!
//! DMA mentransfer data berukuran 16-bit (Halfword) atau 32-bit (Word)
//! dengan berbagai mode alamat (Increment, Decrement, Fixed, Increment & Reload)
//! dan pemicu start timing (Immediate, VBlank, HBlank, Special).

/// Mode perubahan alamat tujuan DMA
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestAddrControl {
    Increment = 0,
    Decrement = 1,
    Fixed = 2,
    IncrementReload = 3,
}

impl From<u16> for DestAddrControl {
    fn from(val: u16) -> Self {
        match val & 0x03 {
            0 => DestAddrControl::Increment,
            1 => DestAddrControl::Decrement,
            2 => DestAddrControl::Fixed,
            _ => DestAddrControl::IncrementReload,
        }
    }
}

/// Mode perubahan alamat sumber DMA
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SrcAddrControl {
    Increment = 0,
    Decrement = 1,
    Fixed = 2,
    Reserved = 3,
}

impl From<u16> for SrcAddrControl {
    fn from(val: u16) -> Self {
        match val & 0x03 {
            0 => SrcAddrControl::Increment,
            1 => SrcAddrControl::Decrement,
            2 => SrcAddrControl::Fixed,
            _ => SrcAddrControl::Reserved,
        }
    }
}

/// Pemicu waktu mulai transfer DMA (Start Timing)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmaTiming {
    Immediate = 0,
    VBlank = 1,
    HBlank = 2,
    Special = 3,
}

impl From<u16> for DmaTiming {
    fn from(val: u16) -> Self {
        match val & 0x03 {
            0 => DmaTiming::Immediate,
            1 => DmaTiming::VBlank,
            2 => DmaTiming::HBlank,
            _ => DmaTiming::Special,
        }
    }
}

/// Satu channel DMA GBA
#[derive(Clone, Debug)]
pub struct DmaChannel {
    pub id: usize,

    // Register yang ditulis oleh CPU
    pub sad: u32,
    pub dad: u32,
    pub count_raw: u16,
    pub cnt_h: u16,

    // Internal working registers yang di-latch saat DMA di-enable
    pub internal_src: u32,
    pub internal_dst: u32,
    pub enabled: bool,
}

impl DmaChannel {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            sad: 0,
            dad: 0,
            count_raw: 0,
            cnt_h: 0,
            internal_src: 0,
            internal_dst: 0,
            enabled: false,
        }
    }

    pub fn reset(&mut self) {
        self.sad = 0;
        self.dad = 0;
        self.count_raw = 0;
        self.cnt_h = 0;
        self.internal_src = 0;
        self.internal_dst = 0;
        self.enabled = false;
    }

    #[inline(always)]
    pub fn dest_addr_control(&self) -> DestAddrControl {
        DestAddrControl::from((self.cnt_h >> 5) & 0x03)
    }

    #[inline(always)]
    pub fn src_addr_control(&self) -> SrcAddrControl {
        SrcAddrControl::from((self.cnt_h >> 7) & 0x03)
    }

    #[inline(always)]
    pub fn repeat(&self) -> bool {
        (self.cnt_h & (1 << 9)) != 0
    }

    #[inline(always)]
    pub fn is_32bit(&self) -> bool {
        (self.cnt_h & (1 << 10)) != 0
    }

    #[inline(always)]
    pub fn timing(&self) -> DmaTiming {
        DmaTiming::from((self.cnt_h >> 12) & 0x03)
    }

    #[inline(always)]
    pub fn irq_enable(&self) -> bool {
        (self.cnt_h & (1 << 14)) != 0
    }

    /// Menghitung jumlah transfer word/halfword
    pub fn transfer_count(&self) -> u32 {
        let max_cnt = if self.id == 3 { 0x10000 } else { 0x4000 };
        let cnt = (self.count_raw as u32) & (max_cnt - 1);
        if cnt == 0 {
            max_cnt
        } else {
            cnt
        }
    }

    /// Tulis register kontrol DMAnCNT_H.
    /// Mengembalikan true jika channel diaktifkan dan pemicunya Immediate (harus langsung ditransfer).
    pub fn write_cnt_h(&mut self, val: u16) -> bool {
        self.cnt_h = val;
        let new_enabled = (val & (1 << 15)) != 0;

        if new_enabled {
            self.internal_src = self.sad;
            self.internal_dst = self.dad;
            self.enabled = true;
            self.timing() == DmaTiming::Immediate
        } else {
            self.enabled = false;
            false
        }
    }

    /// Baca nilai DMAnCNT_H (bit 15 mencerminkan status enabled terkini)
    pub fn read_cnt_h(&self) -> u16 {
        (self.cnt_h & !0x8000) | ((self.enabled as u16) << 15)
    }

    /// Tulis DMAnSAD (masking sesuai channel)
    pub fn write_sad_low(&mut self, val: u16) {
        let mask = if self.id == 0 { 0x07FF_FFFF } else { 0x0FFF_FFFF };
        self.sad = ((self.sad & 0xFFFF_0000) | (val as u32)) & mask;
    }

    pub fn write_sad_high(&mut self, val: u16) {
        let mask = if self.id == 0 { 0x07FF_FFFF } else { 0x0FFF_FFFF };
        self.sad = ((self.sad & 0x0000_FFFF) | ((val as u32) << 16)) & mask;
    }

    /// Tulis DMAnDAD (masking sesuai channel)
    pub fn write_dad_low(&mut self, val: u16) {
        let mask = if self.id == 3 { 0x0FFF_FFFF } else { 0x07FF_FFFF };
        self.dad = ((self.dad & 0xFFFF_0000) | (val as u32)) & mask;
    }

    pub fn write_dad_high(&mut self, val: u16) {
        let mask = if self.id == 3 { 0x0FFF_FFFF } else { 0x07FF_FFFF };
        self.dad = ((self.dad & 0x0000_FFFF) | ((val as u32) << 16)) & mask;
    }

    /// Tulis DMAnCNT_L
    pub fn write_cnt_l(&mut self, val: u16) {
        let mask = if self.id == 3 { 0xFFFF } else { 0x3FFF };
        self.count_raw = val & mask;
    }
}

/// Controller DMA untuk 4 channel
pub struct DmaController {
    pub channels: [DmaChannel; 4],
}

impl Default for DmaController {
    fn default() -> Self {
        Self::new()
    }
}

impl DmaController {
    pub fn new() -> Self {
        Self {
            channels: [
                DmaChannel::new(0),
                DmaChannel::new(1),
                DmaChannel::new(2),
                DmaChannel::new(3),
            ],
        }
    }

    pub fn reset(&mut self) {
        for ch in &mut self.channels {
            ch.reset();
        }
    }

    pub fn step(&mut self, _cycles: usize) {
        // Siklus DMA ditangani secara sinkron pada bus transfer saat pemicu aktif
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dma_channel_address_masking() {
        let mut dma0 = DmaChannel::new(0);
        dma0.write_sad_low(0xFFFF);
        dma0.write_sad_high(0xFFFF);
        assert_eq!(dma0.sad, 0x07FF_FFFF); // 27-bit mask

        let mut dma3 = DmaChannel::new(3);
        dma3.write_sad_low(0xFFFF);
        dma3.write_sad_high(0xFFFF);
        assert_eq!(dma3.sad, 0x0FFF_FFFF); // 28-bit mask

        dma0.write_dad_low(0xFFFF);
        dma0.write_dad_high(0xFFFF);
        assert_eq!(dma0.dad, 0x07FF_FFFF); // 27-bit mask

        dma3.write_dad_low(0xFFFF);
        dma3.write_dad_high(0xFFFF);
        assert_eq!(dma3.dad, 0x0FFF_FFFF); // 28-bit mask
    }

    #[test]
    fn test_dma_channel_counts() {
        let mut dma0 = DmaChannel::new(0);
        dma0.write_cnt_l(0);
        assert_eq!(dma0.transfer_count(), 0x4000); // 0 diartikan max 16384

        dma0.write_cnt_l(100);
        assert_eq!(dma0.transfer_count(), 100);

        let mut dma3 = DmaChannel::new(3);
        dma3.write_cnt_l(0);
        assert_eq!(dma3.transfer_count(), 0x10000); // 0 diartikan max 65536
    }

    #[test]
    fn test_dma_control_bits_and_immediate_trigger() {
        let mut dma = DmaChannel::new(1);
        dma.sad = 0x0200_0000;
        dma.dad = 0x0600_0000;
        dma.write_cnt_l(32);

        // Bit 15: Enable, Bit 12-13: 00b (Immediate), Bit 10: 1 (32-bit), Bit 14: 1 (IRQ Enable)
        let control_val = (1 << 15) | (1 << 14) | (1 << 10);
        let immediate_run = dma.write_cnt_h(control_val);

        assert!(immediate_run);
        assert!(dma.enabled);
        assert!(dma.is_32bit());
        assert!(dma.irq_enable());
        assert_eq!(dma.timing(), DmaTiming::Immediate);
        assert_eq!(dma.internal_src, 0x0200_0000);
        assert_eq!(dma.internal_dst, 0x0600_0000);
    }
}
