use crate::apu::Apu;
use crate::bus::GbaBus;
use crate::cpu::cpsr::CpuMode;
use crate::cpu::registers::REG_SP;
use crate::cpu::Arm7Tdmi;
use crate::dma::DmaTiming;

/// Representasi kesatuan sistem konsol Game Boy Advance
pub struct Gba {
    pub cpu: Arm7Tdmi,
    pub bus: GbaBus,
}

impl Default for Gba {
    fn default() -> Self {
        Self::new()
    }
}

impl Gba {
    pub fn new() -> Self {
        Self {
            cpu: Arm7Tdmi::new(),
            bus: GbaBus::new(),
        }
    }

    #[inline(always)]
    pub fn apu(&self) -> &Apu {
        &self.bus.apu
    }

    #[inline(always)]
    pub fn apu_mut(&mut self) -> &mut Apu {
        &mut self.bus.apu
    }

    /// Reset state konsol dan lakukan bootstrap pipeline
    pub fn reset(&mut self) {
        self.cpu.regs.reset();
        self.cpu.halted = false;
        self.bus.init_default_bios();

        // Jika Cartridge ROM dimuat dan BIOS HLE aktif (tidak ada berkas BIOS eksternal),
        // inisialisasi state register CPU ke post-boot state resmi GBA:
        // PC = 0x0800_0000, System Mode, Stack Pointer diset.
        if !self.bus.rom.is_empty() && self.bus.bios_hle {
            self.cpu.regs.set_pc(0x0800_0000);
            self.cpu.regs.change_mode(CpuMode::System);
            self.cpu.regs.cpsr.set_thumb(false);
            self.cpu.regs.cpsr.set_irq_disabled(false);
            self.cpu.regs.set(REG_SP, 0x0300_7F00);
        }

        self.cpu.refill_pipeline(&mut self.bus);
    }

    /// Jalankan satu langkah (step) eksekusi instruksi CPU serta
    /// lakukan sinkronisasi siklus waktu (cycle-stepping) ke PPU, Timer, APU, dan DMA.
    pub fn step(&mut self) -> usize {
        let cycles = self.cpu.step(&mut self.bus);

        // Majukan siklus PPU dan periksa pemicuan IRQ serta DMA
        let irq_events = self.bus.ppu.step(
            cycles,
            self.bus.vram.as_slice(),
            self.bus.palram.as_slice(),
            self.bus.oam.as_slice(),
        );
        if irq_events.vblank {
            self.bus.interrupts.request_irq(0x0001);
            self.bus.trigger_dma(DmaTiming::VBlank);
        }
        if irq_events.hblank {
            self.bus.interrupts.request_irq(0x0002);
            self.bus.trigger_dma(DmaTiming::HBlank);
        }
        if irq_events.vcount {
            self.bus.interrupts.request_irq(0x0004);
        }

        // Majukan Timers dan tangani overflow IRQ serta audio Direct Sound
        let _timer_events = self.bus.step_timers(cycles);

        self.bus.apu.step(cycles);

        if self.bus.interrupts.has_pending_irq() {
            self.cpu.trigger_irq(&mut self.bus);
        }

        cycles
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;
    use crate::cpu::cpsr::CpuMode;

    #[test]
    fn test_gba_full_system_cpu_ppu_interrupt_loop() {
        let mut gba = Gba::new();

        // Tulis program ARM kecil ke BIOS/IWRAM di 0x0000_0000:
        // 1. MOV R0, #0x04000000 -> 0xE3A00404 (0x04 di-rotate right 8 = 0x04000000)
        // 2. LDR R1, =0x0403 (Mode 3 + BG2 enable) -> kita buat dengan MOV + ORR atau LDR
        //    MOV R1, #0x03 -> 0xE3A01003
        //    ORR R1, R1, #0x0400 -> 0xE3811B01 (0x01 ROR 22 = 0x0400)
        // 3. STRH R1, [R0] (Tulis DISPCNT) -> 0xE1C010B0
        // 4. MOV R2, #0x08 (DISPSTAT VBlank IRQ Enable) -> 0xE3A02008
        // 5. STRH R2, [R0, #4] (Tulis DISPSTAT) -> 0xE1C020B4
        // 6. MOV R3, #0x01 (IE VBlank Enable) -> 0xE3A03001
        // 7. STRH R3, [R0, #0x200] -> 0xE5803200 (atau STR) -> 0xE1C030B0 dengan offset
        //    Lebih mudah: langsung test via bus.write untuk setup, lalu eksekusi loop B . di CPU!

        // Setup konfigurasi via CPU register writes:
        gba.bus.write16(0x0400_0000, 0x0403); // DISPCNT: Mode 3 + BG2
        gba.bus.write16(0x0400_0004, 0x0008); // DISPSTAT: VBlank IRQ Enable
        gba.bus.write16(0x0400_0200, 0x0001); // IE: VBlank Interrupt Enable
        gba.bus.write16(0x0400_0208, 0x0001); // IME: 1 (Master Enable)

        // Program di 0x0000_0000: B . (Infinite loop: 0xEAFF_FFFE)
        gba.bus.write32(0x0000_0000, 0xEAFF_FFFE);
        gba.bus.write32(0x0000_0004, 0xEAFF_FFFE);
        gba.reset();

        // Pastikan flag I di CPSR = 0 (IRQ diizinkan)
        gba.cpu.regs.cpsr.set_irq_disabled(false);

        // Jalankan loop emulasi sampai memasuki VBlank (sekitar 160 scanlines * 1232 siklus / 3 siklus per step)
        let mut total_cycles = 0;
        let target_cycles = 160 * 1232 + 500;

        while total_cycles < target_cycles {
            total_cycles += gba.step();
            if gba.cpu.regs.cpsr.mode() == CpuMode::Irq {
                break;
            }
        }

        // Verifikasi hasil integrasi penuh:
        // 1. PPU telah mencapai scanline VBlank (>= 160)
        assert!(gba.bus.ppu.scanline >= 160);
        // 2. Interrupt Flag VBlank telah terpicu
        assert!((gba.bus.interrupts.if_ & 0x0001) != 0);
        // 3. CPU merespons IRQ dan berpindah ke mode IRQ!
        assert_eq!(gba.cpu.regs.cpsr.mode(), CpuMode::Irq);
        // 4. PC CPU telah melompat ke IRQ Vector (0x0000_0018 + pipeline advance)
        assert!(gba.cpu.regs.pc() >= 0x0000_0018);
    }

 
 
 
    #[test]
    fn test_gba_direct_rom_boot_and_swi_div() {
        let mut gba = Gba::new();

        // Buat ROM mini:
        // 1. MOV R0, #100 (0xE3A00064)
        // 2. MOV R1, #7   (0xE3A01007)
        // 3. SWI 0x060000 (0xEF060000) -> Div R0 / R1: R0=14 (0xE), R1=2, R3=14
        // 4. B .          (0xEAFFFFFE) -> Infinite loop
        let mut rom = vec![0u8; 64];
        let instrs = [
            0xE3A0_0064u32, // MOV R0, #100
            0xE3A0_1007u32, // MOV R1, #7
            0xEF06_0000u32, // SWI 0x060000 (Div)
            0xEAFF_FFFEu32, // B .
        ];
        for (i, &instr) in instrs.iter().enumerate() {
            rom[i * 4..(i + 1) * 4].copy_from_slice(&instr.to_le_bytes());
        }

        gba.bus.load_rom(&rom);
        gba.reset();

        // Pastikan PC berada pada 0x0800_0000 setelah reset dengan ROM
        assert_eq!(gba.cpu.regs.pc(), 0x0800_0008); // 0x0800_0000 + pipeline (8 bytes)

        // Step 1: MOV R0, #100
        gba.step();
        assert_eq!(gba.cpu.regs.get(0), 100);

        // Step 2: MOV R1, #7
        gba.step();
        assert_eq!(gba.cpu.regs.get(1), 7);

        // Step 3: SWI 0x06 (Div)
        gba.step();
        assert_eq!(gba.cpu.regs.get(0), 14); // 100 / 7 = 14
        assert_eq!(gba.cpu.regs.get(1), 2);  // 100 % 7 = 2
        assert_eq!(gba.cpu.regs.get(3), 14); // abs(14) = 14
    }

    #[test]
    fn test_gba_swi_cpuset_memory_transfer() {
        let mut gba = Gba::new();

        // Buat data di EWRAM
        gba.bus.write32(0x0200_0000, 0xDEAD_BEEF);

        // Siapkan register untuk SWI 0x0B (CpuSet):
        // R0 = 0x0200_0000 (src)
        // R1 = 0x0300_0000 (dst)
        // R2 = Fixed fill (bit 24) | 32-bit (bit 26) | 4 words = 0x0500_0004
        gba.cpu.regs.set(0, 0x0200_0000);
        gba.cpu.regs.set(1, 0x0300_0000);
        gba.cpu.regs.set(2, (1 << 24) | (1 << 26) | 4);

        // Panggil SWI 0x0B
        crate::bios::hle::handle_swi(&mut gba.cpu, &mut gba.bus, 0x0B);

        // Verifikasi IWRAM telah diisi 4 word 0xDEADBEEF
        assert_eq!(gba.bus.read32(0x0300_0000), 0xDEAD_BEEF);
        assert_eq!(gba.bus.read32(0x0300_0004), 0xDEAD_BEEF);
        assert_eq!(gba.bus.read32(0x0300_0008), 0xDEAD_BEEF);
        assert_eq!(gba.bus.read32(0x0300_000C), 0xDEAD_BEEF);
    }

    #[test]
    fn test_diagnose_pokemon_boot() {
        let rom_path = "/home/haikal/Documents/Gamees/Pokemon-FireRed-Version(USA).gba";
        if let Ok(rom_data) = std::fs::read(rom_path) {
            let mut gba = Gba::new();
            gba.bus.load_rom(&rom_data);
            gba.reset();

            let mut had_visual_pixels = false;
            // Jalankan 30 frame pertama untuk verifikasi boot crt0, setup stack, IRQ, dan rendering awal
            for _frame in 0..30 {
                let mut cycles = 0;
                while cycles < 280896 {
                    let c = gba.step();
                    cycles += c;
                }
                let non_zero_pixels = gba.bus.ppu.frame_buffer.iter().filter(|&&p| p != 0 && p != 0xFF000000).count();
                if non_zero_pixels > 0 {
                    had_visual_pixels = true;
                }
            }

            // Game harus berada dalam Mode 0 (Tilemap engine aktif) dan telah merender frame
            assert_eq!(gba.bus.ppu.dispcnt, 0x0140);
            assert_eq!(gba.cpu.regs.cpsr.mode(), CpuMode::System);
            assert!(had_visual_pixels, "Harus merender visual frame ke framebuffer");
        }
    }
}
