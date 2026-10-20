pub mod memory;

pub use memory::GbaBus;

/// Trait abstraksi bus memori untuk komunikasi antara CPU, DMA, dan komponen periferal.
pub trait Bus {
    /// Baca nilai 8-bit dari alamat tertentu.
    fn read8(&mut self, addr: u32) -> u8;

    /// Baca nilai 16-bit (halfword) dari alamat aligned.
    fn read16(&mut self, addr: u32) -> u16;

    /// Baca nilai 32-bit (word) dari alamat aligned.
    fn read32(&mut self, addr: u32) -> u32;

    /// Tulis nilai 8-bit ke alamat tertentu.
    fn write8(&mut self, addr: u32, val: u8);

    /// Tulis nilai 16-bit (halfword) ke alamat tertentu.
    fn write16(&mut self, addr: u32, val: u16);

    /// Tulis nilai 32-bit (word) ke alamat tertentu.
    fn write32(&mut self, addr: u32, val: u32);

    /// Karakteristik arsitektur ARMv4T: LDR pada alamat unaligned memutar (rotate right)
    /// nilai word sebesar (addr & 3) * 8 bit.
    #[inline(always)]
    fn read32_unaligned(&mut self, addr: u32) -> u32 {
        let aligned = addr & !0x3;
        let val = self.read32(aligned);
        let shift = (addr & 0x3) * 8;
        val.rotate_right(shift)
    }

    /// Karakteristik arsitektur ARMv4T: LDRH unaligned
    #[inline(always)]
    fn read16_unaligned(&mut self, addr: u32) -> u16 {
        let aligned = addr & !0x1;
        let val = self.read16(aligned);
        let shift = (addr & 0x1) * 8;
        val.rotate_right(shift)
    }

    /// Apakah High-Level Emulation (HLE) BIOS aktif saat menangani instruksi SWI
    #[inline(always)]
    fn is_bios_hle(&self) -> bool {
        true
    }
}
