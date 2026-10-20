use super::Bus;
use crate::apu::Apu;
use crate::dma::{DestAddrControl, DmaController, DmaTiming, SrcAddrControl};
use crate::interrupts::InterruptController;
use crate::keypad::Keypad;
use crate::ppu::Ppu;
use crate::timers::{TimerEvents, Timers};

/// Ukuran memori fisik GBA
pub const BIOS_SIZE: usize = 16 * 1024;        // 16 KB
pub const EWRAM_SIZE: usize = 256 * 1024;      // 256 KB (On-board WRAM)
pub const IWRAM_SIZE: usize = 32 * 1024;       // 32 KB (On-chip WRAM - Fast 32-bit bus)
pub const IO_SIZE: usize = 1024;               // 1 KB I/O Registers
pub const PALRAM_SIZE: usize = 1024;           // 1 KB Palette RAM (BG/OBJ)
pub const VRAM_SIZE: usize = 96 * 1024;        // 96 KB Video RAM
pub const OAM_SIZE: usize = 1024;              // 1 KB Object Attribute Memory (Sprites)
pub const SRAM_SIZE: usize = 64 * 1024;        // 64 KB Game Pak SRAM/Flash backup

/// Implementasi Bus Memori GBA yang memetakan seluruh address space (0x0000_0000..=0x0FFF_FFFF)
pub struct GbaBus {
    pub bios: Box<[u8; BIOS_SIZE]>,
    pub ewram: Box<[u8; EWRAM_SIZE]>,
    pub iwram: Box<[u8; IWRAM_SIZE]>,
    pub io: Box<[u8; IO_SIZE]>,
    pub palram: Box<[u8; PALRAM_SIZE]>,
    pub vram: Box<[u8; VRAM_SIZE]>,
    pub oam: Box<[u8; OAM_SIZE]>,
    pub rom: Vec<u8>,
    pub sram: Box<[u8; SRAM_SIZE]>,

    pub ppu: Ppu,
    pub interrupts: InterruptController,
    pub keypad: Keypad,
    pub dma: DmaController,
    pub timers: Timers,
    pub apu: Apu,

    /// Menyimpan nilai open bus terakhir (berguna ketika membaca memori kosong / unmapped)
    pub open_bus: u32,

    /// Flag apakah High-Level Emulation (HLE) BIOS aktif
    pub bios_hle: bool,
}

impl Default for GbaBus {
    fn default() -> Self {
        Self::new()
    }
}

impl GbaBus {
    pub fn new() -> Self {
        let mut bus = Self {
            bios: Box::new([0; BIOS_SIZE]),
            ewram: Box::new([0; EWRAM_SIZE]),
            iwram: Box::new([0; IWRAM_SIZE]),
            io: Box::new([0; IO_SIZE]),
            palram: Box::new([0; PALRAM_SIZE]),
            vram: Box::new([0; VRAM_SIZE]),
            oam: Box::new([0; OAM_SIZE]),
            rom: Vec::new(),
            sram: Box::new([0; SRAM_SIZE]),
            ppu: Ppu::new(),
            interrupts: InterruptController::new(),
            keypad: Keypad::new(),
            dma: DmaController::new(),
            timers: Timers::new(),
            apu: Apu::new(),
            open_bus: 0,
            bios_hle: true,
        };
        bus.init_default_bios();
        bus
    }

    /// Inisialisasi BIOS stub default untuk exception vector dan IRQ dispatching
    pub fn init_default_bios(&mut self) {
        let mut write_word = |addr: usize, val: u32| {
            self.bios[addr..addr + 4].copy_from_slice(&val.to_le_bytes());
        };

        // 0x0000_0000: Reset Vector -> LDR PC, [PC, #0x18] (loads 0x0800_0000)
        write_word(0x0000, 0xE59F_F018);
        write_word(0x0020, 0x0800_0000);

        // 0x0000_0018: Hardware IRQ Vector -> B 0x0128
        // Offset: (0x128 - (0x18 + 8)) / 4 = 0x108 / 4 = 0x42
        write_word(0x0018, 0xEA00_0042);

        // 0x0000_0128: Standar BIOS IRQ Handler Routine
        // 1. Simpan register scratch ke SP_irq
        write_word(0x0128, 0xE92D_500F); // STMFD SP!, {R0-R3, R12, LR}
        // 2. Ambil user IRQ handler pointer dari IWRAM [0x03007FFC] (mirror [0x03FFFFFC])
        write_word(0x012C, 0xE3A0_0404); // MOV R0, #0x04000000
        write_word(0x0130, 0xE510_1004); // LDR R1, [R0, #-4]
        write_word(0x0134, 0xE351_0000); // CMP R1, #0
        write_word(0x0138, 0x0A00_0002); // BEQ 0x0148 (jika NULL, skip dan langsung restore & exit)
        // 3. Set LR sebagai return address ke 0x0144, lalu panggil user handler
        write_word(0x013C, 0xE28F_E000); // ADD LR, PC, #0 (LR = 0x0144)
        write_word(0x0140, 0xE12F_FF11); // BX R1 (support ARM dan THUMB user handler)
        // 4. Return point setelah user handler selesai (user handler return via BX LR)
        write_word(0x0144, 0xE1A0_0000); // NOP
        // 5. Restore registers dari SP_irq dan return dari IRQ exception
        write_word(0x0148, 0xE8BD_500F); // LDMFD SP!, {R0-R3, R12, LR}
        write_word(0x014C, 0xE25E_F004); // SUBS PC, LR, #4 (restore CPSR dari SPSR)
    }

    /// Muat berkas BIOS resmi atau custom open-source BIOS (seperti Normmatt BIOS)
    pub fn load_bios(&mut self, bios_data: &[u8]) {
        let len = bios_data.len().min(BIOS_SIZE);
        self.bios[..len].copy_from_slice(&bios_data[..len]);
        self.bios_hle = false;
    }

    /// Muat Cartridge Game Pak ROM
    pub fn load_rom(&mut self, rom_data: &[u8]) {
        self.rom = rom_data.to_vec();
    }

    /// Helper untuk VRAM mirroring: VRAM berukuran 96KB (0x18000 bytes).
    /// Jika mengakses 0x06018000..0x0601FFFF, GBA melakukan mirror ke 0x06010000..0x06017FFF.
    #[inline(always)]
    fn vram_mirror_addr(offset: u32) -> usize {
        let mut offset = (offset % 0x20000) as usize;
        if offset >= 0x18000 {
            offset -= 0x8000;
        }
        offset
    }

    /// Baca register I/O 16-bit
    fn read_io_16(&self, offset: u32) -> u16 {
        match offset {
            0x0000 => self.ppu.dispcnt,
            0x0004 => self.ppu.read_dispstat(),
            0x0006 => self.ppu.vcount(),
            0x0008..=0x000E => {
                let bg = ((offset - 0x0008) / 2) as usize;
                self.ppu.bgcnt[bg]
            }
            0x0010..=0x001E => 0, // Scroll offsets are write-only on GBA hardware
            // 0x0080..=0x0088: Sound Control Registers
            0x0080 => self.apu.soundcnt_l,
            0x0082 => self.apu.soundcnt_h,
            0x0084 => self.apu.soundcnt_x,
            0x0088 => self.apu.soundbias,
            // 0x00B0..=0x00DE: DMA 0-3
            0x00B0..=0x00DE => {
                let offset_dma = offset - 0x00B0;
                let ch = (offset_dma / 0x0C) as usize;
                let reg = offset_dma % 0x0C;
                if ch < 4 && reg == 0x0A {
                    self.dma.channels[ch].read_cnt_h()
                } else {
                    0
                }
            }
            // 0x0100..=0x010E: Timers 0-3
            0x0100..=0x010E => {
                let id = ((offset - 0x0100) / 4) as usize;
                if id < 4 {
                    if (offset & 2) == 0 {
                        self.timers.timers[id].read_cnt_l()
                    } else {
                        self.timers.timers[id].read_cnt_h()
                    }
                } else {
                    0
                }
            }
            0x0130 => self.keypad.keyinput,
            0x0132 => self.keypad.keycnt,
            0x0200 => self.interrupts.ie,
            0x0202 => self.interrupts.if_,
            0x0208 => {
                if self.interrupts.ime { 1 } else { 0 }
            }
            _ => {
                let idx = (offset % (IO_SIZE as u32)) as usize;
                let b0 = self.io[idx];
                let b1 = self.io[idx + 1];
                u16::from_le_bytes([b0, b1])
            }
        }
    }

    /// Tulis register I/O 16-bit
    fn write_io_16(&mut self, offset: u32, val: u16) {
        match offset {
            0x0000 => self.ppu.set_dispcnt(val),
            0x0004 => self.ppu.set_dispstat(val),
            0x0006 => {} // VCOUNT is read-only
            0x0008..=0x000E => {
                let bg = ((offset - 0x0008) / 2) as usize;
                self.ppu.bgcnt[bg] = val;
            }
            0x0010..=0x001E => {
                let bg = ((offset - 0x0010) / 4) as usize;
                if (offset & 2) == 0 {
                    self.ppu.bghofs[bg] = val & 0x01FF; // 9-bit mask
                } else {
                    self.ppu.bgvofs[bg] = val & 0x01FF;
                }
            }
            // 0x0060..=0x0064: Channel 1 PSG
            0x0060 => self.apu.ch1.write_sweep(val),
            0x0062 => self.apu.ch1.write_cnt_h(val),
            0x0064 => self.apu.ch1.write_cnt_x(val),
            // 0x0080..=0x0088: Sound Control Registers
            0x0080 => self.apu.soundcnt_l = val,
            0x0082 => self.apu.write_soundcnt_h(val),
            0x0084 => self.apu.soundcnt_x = (self.apu.soundcnt_x & 0x007F) | (val & 0x0080),
            0x0088 => self.apu.soundbias = val,
            // 0x00B0..=0x00DE: DMA 0-3
            0x00B0..=0x00DE => {
                let offset_dma = offset - 0x00B0;
                let ch = (offset_dma / 0x0C) as usize;
                let reg = offset_dma % 0x0C;
                let mut run_immediate = false;
                if ch < 4 {
                    match reg {
                        0x00 => self.dma.channels[ch].write_sad_low(val),
                        0x02 => self.dma.channels[ch].write_sad_high(val),
                        0x04 => self.dma.channels[ch].write_dad_low(val),
                        0x06 => self.dma.channels[ch].write_dad_high(val),
                        0x08 => self.dma.channels[ch].write_cnt_l(val),
                        0x0A => {
                            run_immediate = self.dma.channels[ch].write_cnt_h(val);
                        }
                        _ => {}
                    }
                }
                if run_immediate {
                    self.execute_dma(ch);
                }
            }
            // 0x0100..=0x010E: Timers 0-3
            0x0100..=0x010E => {
                let id = ((offset - 0x0100) / 4) as usize;
                if id < 4 {
                    if (offset & 2) == 0 {
                        self.timers.timers[id].write_cnt_l(val);
                    } else {
                        self.timers.timers[id].write_cnt_h(val);
                    }
                }
            }
            0x0130 => {} // KEYINPUT is read-only for software
            0x0132 => self.keypad.keycnt = val,
            0x0200 => self.interrupts.ie = val,
            // Acknowledge IRQ: Menulis bit 1 membersihkan request interrupt terkait
            0x0202 => self.interrupts.if_ &= !val,
            0x0208 => self.interrupts.ime = (val & 1) != 0,
            _ => {
                let idx = (offset % (IO_SIZE as u32)) as usize;
                let [b0, b1] = val.to_le_bytes();
                self.io[idx] = b0;
                self.io[idx + 1] = b1;
            }
        }
    }

    /// Menjalankan satu channel DMA dan mengembalikan jumlah siklus bus yang dikonsumsi
    pub fn execute_dma(&mut self, ch: usize) -> usize {
        let (src, dst, count, is_32bit, src_ctl, dst_ctl, repeat, irq) = {
            let channel = &mut self.dma.channels[ch];
            if !channel.enabled {
                return 0;
            }

            // Reload destination jika mode IncrementReload
            if channel.dest_addr_control() == DestAddrControl::IncrementReload {
                channel.internal_dst = channel.dad;
            }

            let is_special_fifo = (ch == 1 || ch == 2) && channel.timing() == DmaTiming::Special;
            let count = if is_special_fifo {
                4
            } else {
                channel.transfer_count()
            };
            let is_32bit = if is_special_fifo { true } else { channel.is_32bit() };
            let dst = if is_special_fifo {
                if ch == 1 { 0x0400_00A0 } else { 0x0400_00A4 }
            } else {
                channel.internal_dst
            };

            let dst_ctl = if is_special_fifo {
                DestAddrControl::Fixed
            } else {
                channel.dest_addr_control()
            };

            (
                channel.internal_src,
                dst,
                count,
                is_32bit,
                channel.src_addr_control(),
                dst_ctl,
                channel.repeat(),
                channel.irq_enable(),
            )
        };

        let step = if is_32bit { 4 } else { 2 };
        let mut cur_src = src;
        let mut cur_dst = dst;

        if is_32bit {
            cur_src &= !3;
            cur_dst &= !3;
            for _ in 0..count {
                let val = self.read32(cur_src);
                self.write32(cur_dst, val);

                match src_ctl {
                    SrcAddrControl::Increment | SrcAddrControl::Reserved => cur_src = cur_src.wrapping_add(step),
                    SrcAddrControl::Decrement => cur_src = cur_src.wrapping_sub(step),
                    SrcAddrControl::Fixed => {}
                }

                match dst_ctl {
                    DestAddrControl::Increment | DestAddrControl::IncrementReload => cur_dst = cur_dst.wrapping_add(step),
                    DestAddrControl::Decrement => cur_dst = cur_dst.wrapping_sub(step),
                    DestAddrControl::Fixed => {}
                }
            }
        } else {
            cur_src &= !1;
            cur_dst &= !1;
            for _ in 0..count {
                let val = self.read16(cur_src);
                self.write16(cur_dst, val);

                match src_ctl {
                    SrcAddrControl::Increment | SrcAddrControl::Reserved => cur_src = cur_src.wrapping_add(step),
                    SrcAddrControl::Decrement => cur_src = cur_src.wrapping_sub(step),
                    SrcAddrControl::Fixed => {}
                }

                match dst_ctl {
                    DestAddrControl::Increment | DestAddrControl::IncrementReload => cur_dst = cur_dst.wrapping_add(step),
                    DestAddrControl::Decrement => cur_dst = cur_dst.wrapping_sub(step),
                    DestAddrControl::Fixed => {}
                }
            }
        }

        let channel = &mut self.dma.channels[ch];
        channel.internal_src = cur_src;
        if channel.dest_addr_control() != DestAddrControl::IncrementReload {
            channel.internal_dst = cur_dst;
        }

        let is_immediate = channel.timing() == DmaTiming::Immediate;
        if !repeat || is_immediate {
            channel.enabled = false;
            channel.cnt_h &= !0x8000;
        }

        if irq {
            self.interrupts.request_irq(1 << (8 + ch));
        }

        (count as usize) * 2 + 2
    }

    /// Memicu DMA berdasarkan event waktu (VBlank, HBlank, Special)
    pub fn trigger_dma(&mut self, timing: DmaTiming) -> usize {
        let mut total_cycles = 0;
        for ch in 0..4 {
            if self.dma.channels[ch].enabled && self.dma.channels[ch].timing() == timing {
                total_cycles += self.execute_dma(ch);
            }
        }
        total_cycles
    }

    /// Majukan siklus waktu timer dan tangani request IRQ serta Direct Sound
    pub fn step_timers(&mut self, cycles: usize) -> TimerEvents {
        let events = self.timers.step(cycles);
        for id in 0..4 {
            if events.overflows[id] {
                if self.timers.timers[id].irq_enable() {
                    self.interrupts.request_irq(1 << (3 + id));
                }

                // Clock Direct Sound FIFO
                let dma_req = self.apu.on_timer_overflow(id);
                if (dma_req & 1) != 0 && self.dma.channels[1].enabled && self.dma.channels[1].timing() == DmaTiming::Special {
                    self.execute_dma(1);
                }
                if (dma_req & 2) != 0 && self.dma.channels[2].enabled && self.dma.channels[2].timing() == DmaTiming::Special {
                    self.execute_dma(2);
                }
            }
        }
        events
    }
}

impl Bus for GbaBus {
    fn read8(&mut self, addr: u32) -> u8 {
        let page = (addr >> 24) & 0x0F;
        let offset = addr & 0x00FF_FFFF;

        let val = match page {
            // 0x00: BIOS (16 KB)
            0x00 => {
                if offset < BIOS_SIZE as u32 {
                    self.bios[offset as usize]
                } else {
                    (self.open_bus & 0xFF) as u8
                }
            }
            // 0x02: EWRAM (256 KB, mirrored hingga 0x02FF_FFFF)
            0x02 => {
                let idx = (offset % (EWRAM_SIZE as u32)) as usize;
                self.ewram[idx]
            }
            // 0x03: IWRAM (32 KB, mirrored hingga 0x03FF_FFFF)
            0x03 => {
                let idx = (offset % (IWRAM_SIZE as u32)) as usize;
                self.iwram[idx]
            }
            // 0x04: I/O Registers (1 KB, mirrored hingga 0x04FF_FFFF)
            0x04 => {
                let half = self.read_io_16(offset & !1);
                if (offset & 1) == 0 {
                    (half & 0xFF) as u8
                } else {
                    (half >> 8) as u8
                }
            }
            // 0x05: Palette RAM (1 KB, mirrored hingga 0x05FF_FFFF)
            0x05 => {
                let idx = (offset % (PALRAM_SIZE as u32)) as usize;
                self.palram[idx]
            }
            // 0x06: VRAM (96 KB)
            0x06 => {
                let idx = Self::vram_mirror_addr(offset);
                self.vram[idx]
            }
            // 0x07: OAM (1 KB, mirrored hingga 0x07FF_FFFF)
            0x07 => {
                let idx = (offset % (OAM_SIZE as u32)) as usize;
                self.oam[idx]
            }
            // 0x08..=0x0D: Game Pak ROM Waitstates 0, 1, 2
            0x08..=0x0D => {
                let rom_offset = (addr & 0x01FF_FFFF) as usize;
                if rom_offset < self.rom.len() {
                    self.rom[rom_offset]
                } else {
                    (self.open_bus & 0xFF) as u8
                }
            }
            // 0x0E..=0x0F: Game Pak SRAM / Flash backup
            0x0E | 0x0F => {
                let idx = (offset % (SRAM_SIZE as u32)) as usize;
                self.sram[idx]
            }
            _ => (self.open_bus & 0xFF) as u8,
        };

        self.open_bus = (self.open_bus & !0xFF) | (val as u32);
        val
    }

    fn read16(&mut self, addr: u32) -> u16 {
        let aligned = addr & !1;
        let page = (aligned >> 24) & 0x0F;
        let offset = aligned & 0x00FF_FFFF;

        let val = if page == 0x04 {
            self.read_io_16(offset)
        } else {
            let b0 = self.read8(aligned);
            let b1 = self.read8(aligned + 1);
            u16::from_le_bytes([b0, b1])
        };

        self.open_bus = (self.open_bus & !0xFFFF) | (val as u32);
        val
    }

    fn read32(&mut self, addr: u32) -> u32 {
        let aligned = addr & !3;
        let page = (aligned >> 24) & 0x0F;
        let offset = aligned & 0x00FF_FFFF;

        let val = if page == 0x04 {
            let low = self.read_io_16(offset) as u32;
            let high = self.read_io_16(offset + 2) as u32;
            low | (high << 16)
        } else {
            let b0 = self.read8(aligned);
            let b1 = self.read8(aligned + 1);
            let b2 = self.read8(aligned + 2);
            let b3 = self.read8(aligned + 3);
            u32::from_le_bytes([b0, b1, b2, b3])
        };

        self.open_bus = val;
        val
    }

    fn write8(&mut self, addr: u32, val: u8) {
        let page = (addr >> 24) & 0x0F;
        let offset = addr & 0x00FF_FFFF;

        match page {
            // BIOS: Read-only
            0x00 => {}
            // EWRAM (256 KB)
            0x02 => {
                let idx = (offset % (EWRAM_SIZE as u32)) as usize;
                self.ewram[idx] = val;
            }
            // IWRAM (32 KB)
            0x03 => {
                let idx = (offset % (IWRAM_SIZE as u32)) as usize;
                self.iwram[idx] = val;
            }
            // I/O Registers
            0x04 => {
                let mut half = self.read_io_16(offset & !1);
                if (offset & 1) == 0 {
                    half = (half & 0xFF00) | (val as u16);
                } else {
                    half = (half & 0x00FF) | ((val as u16) << 8);
                }
                self.write_io_16(offset & !1, half);
            }
            // Hardware Quirk: 8-bit write ke PALRAM dan VRAM diduplikasi menjadi 16-bit halfword
            0x05 => {
                let halfword = (val as u16) | ((val as u16) << 8);
                self.write16(addr & !1, halfword);
            }
            0x06 => {
                let halfword = (val as u16) | ((val as u16) << 8);
                self.write16(addr & !1, halfword);
            }
            0x07 => {
                // 8-bit write ke OAM diabaikan hardware!
            }
            // ROM: Read-only
            0x08..=0x0D => {}
            // SRAM / Flash backup
            0x0E | 0x0F => {
                let idx = (offset % (SRAM_SIZE as u32)) as usize;
                self.sram[idx] = val;
            }
            _ => {}
        }
    }

    fn write16(&mut self, addr: u32, val: u16) {
        let aligned = addr & !1;
        let page = (aligned >> 24) & 0x0F;
        let offset = aligned & 0x00FF_FFFF;

        match page {
            0x02 => {
                let idx = (offset % (EWRAM_SIZE as u32)) as usize;
                let [b0, b1] = val.to_le_bytes();
                self.ewram[idx] = b0;
                self.ewram[idx + 1] = b1;
            }
            0x03 => {
                let idx = (offset % (IWRAM_SIZE as u32)) as usize;
                let [b0, b1] = val.to_le_bytes();
                self.iwram[idx] = b0;
                self.iwram[idx + 1] = b1;
            }
            0x04 => {
                self.write_io_16(offset, val);
            }
            0x05 => {
                let idx = (offset % (PALRAM_SIZE as u32)) as usize;
                let [b0, b1] = val.to_le_bytes();
                self.palram[idx] = b0;
                self.palram[idx + 1] = b1;
            }
            0x06 => {
                let idx = Self::vram_mirror_addr(offset);
                let [b0, b1] = val.to_le_bytes();
                self.vram[idx] = b0;
                self.vram[idx + 1] = b1;
            }
            0x07 => {
                let idx = (offset % (OAM_SIZE as u32)) as usize;
                let [b0, b1] = val.to_le_bytes();
                self.oam[idx] = b0;
                self.oam[idx + 1] = b1;
            }
            0x0E | 0x0F => {
                let idx = (offset % (SRAM_SIZE as u32)) as usize;
                self.sram[idx] = val as u8;
            }
            _ => {}
        }
    }

    fn write32(&mut self, addr: u32, val: u32) {
        let aligned = addr & !3;
        let page = (aligned >> 24) & 0x0F;
        let offset = aligned & 0x00FF_FFFF;

        match page {
            0x02 => {
                let idx = (offset % (EWRAM_SIZE as u32)) as usize;
                self.ewram[idx..idx + 4].copy_from_slice(&val.to_le_bytes());
            }
            0x03 => {
                let idx = (offset % (IWRAM_SIZE as u32)) as usize;
                self.iwram[idx..idx + 4].copy_from_slice(&val.to_le_bytes());
            }
            0x04 => {
                if offset == 0x00A0 {
                    self.apu.fifo_a.push_word(val);
                } else if offset == 0x00A4 {
                    self.apu.fifo_b.push_word(val);
                } else {
                    self.write_io_16(offset, val as u16);
                    self.write_io_16(offset + 2, (val >> 16) as u16);
                }
            }
            0x05 => {
                let idx = (offset % (PALRAM_SIZE as u32)) as usize;
                self.palram[idx..idx + 4].copy_from_slice(&val.to_le_bytes());
            }
            0x06 => {
                let idx = Self::vram_mirror_addr(offset);
                self.vram[idx..idx + 4].copy_from_slice(&val.to_le_bytes());
            }
            0x07 => {
                let idx = (offset % (OAM_SIZE as u32)) as usize;
                self.oam[idx..idx + 4].copy_from_slice(&val.to_le_bytes());
            }
            _ => {
                self.write16(aligned, val as u16);
                self.write16(aligned + 2, (val >> 16) as u16);
            }
        }
    }

    #[inline(always)]
    fn is_bios_hle(&self) -> bool {
        self.bios_hle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iwram_read_write() {
        let mut bus = GbaBus::new();
        bus.write32(0x0300_0000, 0x1234_5678);
        assert_eq!(bus.read32(0x0300_0000), 0x1234_5678);
        assert_eq!(bus.read16(0x0300_0000), 0x5678);
        assert_eq!(bus.read8(0x0300_0000), 0x78);
        assert_eq!(bus.read8(0x0300_0001), 0x56);

        // Mirroring IWRAM (mirrored every 32KB = 0x8000)
        assert_eq!(bus.read32(0x0300_8000), 0x1234_5678);
    }

    #[test]
    fn test_arm_unaligned_read32_rotation() {
        let mut bus = GbaBus::new();
        bus.write32(0x0200_0000, 0x1234_5678);

        assert_eq!(bus.read32_unaligned(0x0200_0000), 0x1234_5678);
        assert_eq!(bus.read32_unaligned(0x0200_0001), 0x7812_3456);
        assert_eq!(bus.read32_unaligned(0x0200_0002), 0x5678_1234);
        assert_eq!(bus.read32_unaligned(0x0200_0003), 0x3456_7812);
    }

    #[test]
    fn test_palram_8bit_write_duplication() {
        let mut bus = GbaBus::new();
        bus.write8(0x0500_0000, 0xAB);
        assert_eq!(bus.read16(0x0500_0000), 0xABAB);
    }

    #[test]
    fn test_io_dispcnt_and_vcount_registers() {
        let mut bus = GbaBus::new();
        // Tulis DISPCNT ke 0x0403 (Mode 3 + BG2 enable)
        bus.write16(0x0400_0000, 0x0403);
        assert_eq!(bus.read16(0x0400_0000), 0x0403);
        assert_eq!(bus.ppu.dispcnt, 0x0403);

        // Ubah scanline PPU dan baca melalui register VCOUNT (0x0400_0006)
        bus.ppu.scanline = 120;
        assert_eq!(bus.read16(0x0400_0006), 120);

        // Baca 32-bit di 0x0400_0004 menggabungkan DISPSTAT dan VCOUNT
        let val32 = bus.read32(0x0400_0004);
        assert_eq!((val32 >> 16) as u16, 120);
    }

    #[test]
    fn test_io_interrupt_registers() {
        let mut bus = GbaBus::new();
        // Tulis IE (0x0400_0200) = 0x0001 (VBlank Enable)
        bus.write16(0x0400_0200, 0x0001);
        assert_eq!(bus.read16(0x0400_0200), 0x0001);

        // Tulis IME (0x0400_0208) = 1
        bus.write16(0x0400_0208, 1);
        assert!(bus.interrupts.ime);

        // Simulasi request IRQ VBlank
        bus.interrupts.request_irq(0x0001);
        assert_eq!(bus.read16(0x0400_0202), 0x0001);
        assert!(bus.interrupts.has_pending_irq());

        // Acknowledge VBlank IRQ dengan menulis 1 ke IF (0x0400_0202)
        bus.write16(0x0400_0202, 0x0001);
        assert_eq!(bus.read16(0x0400_0202), 0x0000);
        assert!(!bus.interrupts.has_pending_irq());
    }

    #[test]
    fn test_dma_immediate_transfer_ewram_to_iwram() {
        let mut bus = GbaBus::new();

        // Siapkan data di EWRAM
        bus.write32(0x0200_0000, 0x1122_3344);
        bus.write32(0x0200_0004, 0x5566_7788);
        bus.write32(0x0200_0008, 0x99AA_BBCC);
        bus.write32(0x0200_000C, 0xDDEE_FFAA);

        // Konfigurasi DMA Channel 0 via I/O registers:
        // DMA0SAD = 0x0200_0000 (0x0400_00B0)
        bus.write32(0x0400_00B0, 0x0200_0000);
        // DMA0DAD = 0x0300_0000 (0x0400_00B4)
        bus.write32(0x0400_00B4, 0x0300_0000);
        // DMA0CNT_L = 4 words (0x0400_00B8)
        bus.write16(0x0400_00B8, 4);
        // DMA0CNT_H = Enable (bit 15) | 32-bit (bit 10) | Immediate (0) -> 0x8400 (0x0400_00BA)
        bus.write16(0x0400_00BA, 0x8400);

        // Verifikasi bahwa transfer telah selesai secara instan ke IWRAM
        assert_eq!(bus.read32(0x0300_0000), 0x1122_3344);
        assert_eq!(bus.read32(0x0300_0004), 0x5566_7788);
        assert_eq!(bus.read32(0x0300_0008), 0x99AA_BBCC);
        assert_eq!(bus.read32(0x0300_000C), 0xDDEE_FFAA);

        // Channel harus dinonaktifkan (karena repeat = 0)
        assert!(!bus.dma.channels[0].enabled);
        assert_eq!(bus.read16(0x0400_00BA) & 0x8000, 0);
    }

    #[test]
    fn test_dma_vblank_trigger_and_irq() {
        let mut bus = GbaBus::new();

        // Tulis data di EWRAM
        bus.write16(0x0200_0000, 0xCAFE);
        bus.write16(0x0200_0002, 0xBABE);

        // Konfigurasi DMA Channel 1 via 32-bit write ke DMA1CNT (0x0400_00C4):
        // SAD = 0x0200_0000, DAD = 0x0600_0000 (VRAM)
        bus.write32(0x0400_00BC, 0x0200_0000);
        bus.write32(0x0400_00C0, 0x0600_0000);

        // CNT_L = 2 halfwords (0x0002)
        // CNT_H = Enable (0x8000) | IRQ Enable (0x4000) | Timing VBlank (0x1000) = 0xD000
        // Write 32-bit: CNT_L di low, CNT_H di high
        bus.write32(0x0400_00C4, 0xD000_0002);

        // VRAM sebelum VBlank masih 0
        assert_eq!(bus.read16(0x0600_0000), 0);

        // Picu DMA VBlank
        bus.trigger_dma(DmaTiming::VBlank);

        // Data sekarang ada di VRAM
        assert_eq!(bus.read16(0x0600_0000), 0xCAFE);
        assert_eq!(bus.read16(0x0600_0002), 0xBABE);

        // IRQ DMA1 (bit 9 = 0x0200) harus terpicu
        assert!((bus.interrupts.if_ & 0x0200) != 0);
    }

    #[test]
    fn test_timer_irq_and_counter_io_registers() {
        let mut bus = GbaBus::new();

        // Set IE untuk Timer 0 (bit 3 = 0x0008) & Master Enable (IME = 1)
        bus.write16(0x0400_0200, 0x0008);
        bus.write16(0x0400_0208, 1);

        // Tulis TM0CNT_L = 0xFFFD (0x0400_0100)
        bus.write16(0x0400_0100, 0xFFFD);

        // Tulis TM0CNT_H = Start (bit 7) | IRQ Enable (bit 6) | Prescaler /1 = 0x00C0 (0x0400_0102)
        bus.write16(0x0400_0102, 0x00C0);

        // Baca kembali TM0CNT_L -> harus sesuai reload 0xFFFD
        assert_eq!(bus.read16(0x0400_0100), 0xFFFD);

        // Step 2 siklus: counter jadi 0xFFFF
        bus.step_timers(2);
        assert_eq!(bus.read16(0x0400_0100), 0xFFFF);
        assert!(!bus.interrupts.has_pending_irq());

        // Step 1 siklus: overflow ke reload 0xFFFD dan memicu IRQ!
        bus.step_timers(1);
        assert_eq!(bus.read16(0x0400_0100), 0xFFFD);
        assert!((bus.interrupts.if_ & 0x0008) != 0);
        assert!(bus.interrupts.has_pending_irq());
    }

    #[test]
    fn test_direct_sound_dma_audio_pipeline() {
        let mut bus = GbaBus::new();

        // 1. Tulis 16 byte audio PCM ke EWRAM di 0x0200_0000 (4 word: 0x11111111, 0x22222222, ...)
        bus.write32(0x0200_0000, 0x1111_1111);
        bus.write32(0x0200_0004, 0x2222_2222);
        bus.write32(0x0200_0008, 0x3333_3333);
        bus.write32(0x0200_000C, 0x4444_4444);

        // 2. Setup DMA1 untuk Direct Sound A (Special timing = bits 12-13 11b -> 0x3000, Repeat = bit 9 0x0200, 32-bit = bit 10 0x0400, Enable = bit 15 0x8000)
        // DMA1SAD = 0x0200_0000
        bus.write32(0x0400_00BC, 0x0200_0000);
        // DMA1DAD = 0x0400_00A0 (FIFO A)
        bus.write32(0x0400_00C0, 0x0400_00A0);
        // DMA1CNT_H = 0xB600 (Enable + Timing: Special + 32-bit + Repeat)
        bus.write16(0x0400_00C6, 0xB600);

        // 3. Konfigurasi SOUNDCNT_H: Enable Direct Sound A Left & Right (bits 8 & 9 -> 0x0300), Timer 0 (bit 10 = 0)
        bus.write16(0x0400_0082, 0x0304);

        // FIFO A awalnya kosong (count = 0)
        assert_eq!(bus.apu.fifo_a.count, 0);

        // 4. Konfigurasi Timer 0: reload = 0xFFFF, start (bit 7)
        bus.write16(0x0400_0100, 0xFFFF);
        bus.write16(0x0400_0102, 0x0080);

        // 5. Majukan timer 1 siklus -> Timer 0 overflow!
        // APU on_timer_overflow(0) mendeteksi FIFO A kosong (count <= 16) dan meminta DMA1!
        // Bus otomatis mengeksekusi DMA1 Special, mentransfer 4 word (16 byte) dari EWRAM ke FIFO A!
        bus.step_timers(1);

        // Verifikasi: FIFO A sekarang terisi penuh dengan 16 sampel dari EWRAM!
        assert_eq!(bus.apu.fifo_a.count, 16);
        assert_eq!(bus.apu.fifo_a.pop_sample(), 0x11);
        assert_eq!(bus.apu.fifo_a.pop_sample(), 0x11);
    }
}
