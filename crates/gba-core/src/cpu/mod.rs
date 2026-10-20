pub mod alu;
pub mod arm;
pub mod cpsr;
pub mod registers;
pub mod thumb;

use crate::bus::Bus;
use cpsr::CpuMode;
use registers::{Registers, REG_LR};

/// ARM7TDMI Core Processor (ARMv4T architecture)
pub struct Arm7Tdmi {
    /// Kumpulan register R0-R15 dan CPSR/SPSR
    pub regs: Registers,

    /// 3-stage instruction pipeline buffer:
    /// pipeline[0] = instruksi yang sedang di-decode (akan dieksekusi berikutnya)
    /// pipeline[1] = instruksi yang sedang di-fetch dari bus memori
    pub pipeline: [u32; 2],

    /// Total siklus clock yang telah dieksekusi oleh core
    pub cycles: usize,

    /// Flag penanda apakah branch/exception terjadi pada instruksi saat ini
    pub branched: bool,

    /// Flag CPU halt (SWI 0x02 Halt atau SWI 0x05 VBlankIntrWait)
    pub halted: bool,
}

impl Default for Arm7Tdmi {
    fn default() -> Self {
        Self::new()
    }
}

impl Arm7Tdmi {
    pub fn new() -> Self {
        Self {
            regs: Registers::new(),
            pipeline: [0; 2],
            cycles: 0,
            branched: false,
            halted: false,
        }
    }

    /// Inisialisasi pipeline saat CPU pertama kali boot atau setelah branch/exception.
    /// Membaca 2 instruksi berturut-turut untuk mengisi pipeline stage Decode dan Execute.
    pub fn refill_pipeline(&mut self, bus: &mut impl Bus) {
        self.branched = true;
        if self.regs.cpsr.is_thumb() {
            let pc = self.regs.pc() & !0x1;
            self.pipeline[0] = bus.read16(pc) as u32;
            self.regs.set_pc(pc.wrapping_add(2));

            let pc = self.regs.pc() & !0x1;
            self.pipeline[1] = bus.read16(pc) as u32;
            self.regs.set_pc(pc.wrapping_add(2));
        } else {
            let pc = self.regs.pc() & !0x3;
            self.pipeline[0] = bus.read32(pc);
            self.regs.set_pc(pc.wrapping_add(4));

            let pc = self.regs.pc() & !0x3;
            self.pipeline[1] = bus.read32(pc);
            self.regs.set_pc(pc.wrapping_add(4));
        }
    }

    /// Eksekusi satu instruksi CPU (Fetch -> Decode -> Execute)
    pub fn step(&mut self, bus: &mut impl Bus) -> usize {
        if self.halted {
            self.cycles += 1;
            return 1;
        }

        let is_thumb = self.regs.cpsr.is_thumb();
        let current_instruction = self.pipeline[0];
        self.branched = false;

        let cycles = if is_thumb {
            thumb::execute_thumb(self, bus, current_instruction as u16)
        } else {
            arm::execute_arm(self, bus, current_instruction)
        };

        // Jika instruksi tidak melakukan branch, geser pipeline satu langkah
        if !self.branched {
            self.pipeline[0] = self.pipeline[1];
            if self.regs.cpsr.is_thumb() {
                let pc = self.regs.pc() & !0x1;
                self.pipeline[1] = bus.read16(pc) as u32;
                self.regs.set_pc(pc.wrapping_add(2));
            } else {
                let pc = self.regs.pc() & !0x3;
                self.pipeline[1] = bus.read32(pc);
                self.regs.set_pc(pc.wrapping_add(4));
            }
        }

        self.cycles += cycles;
        cycles
    }

    /// Memicu hardware IRQ (Interrupt Request) jika flag I pada CPSR bernilai 0 (enabled)
    pub fn trigger_irq(&mut self, bus: &mut impl Bus) {
        if self.regs.cpsr.irq_disabled() {
            return;
        }

        // Bangunkan CPU dari keadaan Halt
        self.halted = false;

        // Simpan CPSR saat ini ke SPSR mode IRQ
        let cpsr = self.regs.cpsr.value;
        let is_thumb = self.regs.cpsr.is_thumb();

        // Ganti ke mode IRQ
        self.regs.change_mode(CpuMode::Irq);
        self.regs.set_spsr(cpsr);

        // Hitung return address untuk exception IRQ:
        // Saat exception handler selesai, ia mengeksekusi `SUBS PC, LR, #4`:
        // Di ARM: PC = next_inst + 8, return_pc = PC - 4 (sehingga LR - 4 = next_inst)
        // Di THUMB: PC = next_inst + 4, return_pc = PC (sehingga LR - 4 = next_inst)
        let return_pc = if is_thumb {
            self.regs.pc()
        } else {
            self.regs.pc().wrapping_sub(4)
        };
        self.regs.set(REG_LR, return_pc);

        // Set mode ke ARM (T = 0) dan matikan interrupt (I = 1)
        self.regs.cpsr.set_thumb(false);
        self.regs.cpsr.set_irq_disabled(true);

        // Selalu lompat ke BIOS IRQ vector di 0x0000_0018
        self.regs.set_pc(0x0000_0018);
        self.refill_pipeline(bus);
    }
}
