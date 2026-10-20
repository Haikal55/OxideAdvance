use super::cpsr::{CpuMode, Cpsr};

pub const REG_SP: usize = 13;
pub const REG_LR: usize = 14;
pub const REG_PC: usize = 15;

/// Representasi register CPU ARM7TDMI.
/// Menggunakan sistem active-register array R0-R15 untuk menjamin O(1) direct access
/// tanpa conditional check selama eksekusi instruksi, dan melakukan bank swapping
/// saat terjadi perpindahan mode operasi CPU.
#[derive(Clone, Debug)]
pub struct Registers {
    /// Register aktif R0-R15
    pub r: [u32; 16],
    /// Current Program Status Register
    pub cpsr: Cpsr,

    // Banked registers untuk R8-R12
    r8_12_usr: [u32; 5],
    r8_12_fiq: [u32; 5],

    // Banked registers untuk R13 (SP) & R14 (LR):
    // Index: 0=User/System, 1=Fiq, 2=Irq, 3=Supervisor, 4=Abort, 5=Undefined
    r13_bank: [u32; 6],
    r14_bank: [u32; 6],

    // Saved Program Status Register (SPSR) untuk masing-masing mode privilege
    spsr_bank: [u32; 6],
}

impl Default for Registers {
    fn default() -> Self {
        Self::new()
    }
}

impl Registers {
    pub fn new() -> Self {
        let mut regs = Self {
            r: [0; 16],
            cpsr: Cpsr::new(CpuMode::System as u32),
            r8_12_usr: [0; 5],
            r8_12_fiq: [0; 5],
            r13_bank: [0; 6],
            r14_bank: [0; 6],
            spsr_bank: [0; 6],
        };
        regs.reset();
        regs
    }

    /// Reset registers ke initial state saat GBA dinyalakan (boot state)
    pub fn reset(&mut self) {
        self.r = [0; 16];
        self.cpsr = Cpsr::new(CpuMode::System as u32);
        // Default GBA stack pointer saat masuk System mode
        self.r[REG_SP] = 0x0300_7F00;
        self.r13_bank[CpuMode::System.bank_index()] = 0x0300_7F00;
        self.r13_bank[CpuMode::Irq.bank_index()] = 0x0300_7FA0;
        self.r13_bank[CpuMode::Supervisor.bank_index()] = 0x0300_7FE0;
    }

    #[inline(always)]
    pub fn get(&self, reg: usize) -> u32 {
        self.r[reg]
    }

    #[inline(always)]
    pub fn set(&mut self, reg: usize, val: u32) {
        self.r[reg] = val;
    }

    #[inline(always)]
    pub fn pc(&self) -> u32 {
        self.r[REG_PC]
    }

    #[inline(always)]
    pub fn set_pc(&mut self, val: u32) {
        self.r[REG_PC] = val;
    }

    /// Ambil nilai SPSR untuk mode aktif saat ini
    pub fn get_spsr(&self) -> u32 {
        let mode = self.cpsr.mode();
        self.spsr_bank[mode.bank_index()]
    }

    /// Simpan nilai ke SPSR mode aktif
    pub fn set_spsr(&mut self, val: u32) {
        let mode = self.cpsr.mode();
        if mode != CpuMode::User && mode != CpuMode::System {
            self.spsr_bank[mode.bank_index()] = val;
        }
    }

    /// Lakukan bank swapping saat mode CPU berubah (misal: interrupt IRQ atau instruksi MSR)
    pub fn change_mode(&mut self, new_mode: CpuMode) {
        let old_mode = self.cpsr.mode();
        if old_mode == new_mode {
            return;
        }

        let old_idx = old_mode.bank_index();
        let new_idx = new_mode.bank_index();

        // 1. Simpan SP (R13) dan LR (R14) mode lama, muat untuk mode baru
        self.r13_bank[old_idx] = self.r[REG_SP];
        self.r14_bank[old_idx] = self.r[REG_LR];

        self.r[REG_SP] = self.r13_bank[new_idx];
        self.r[REG_LR] = self.r14_bank[new_idx];

        // 2. Swapping R8-R12 jika melibatkan mode FIQ
        if old_mode == CpuMode::Fiq {
            self.r8_12_fiq.copy_from_slice(&self.r[8..13]);
            self.r[8..13].copy_from_slice(&self.r8_12_usr);
        } else if new_mode == CpuMode::Fiq {
            self.r8_12_usr.copy_from_slice(&self.r[8..13]);
            self.r[8..13].copy_from_slice(&self.r8_12_fiq);
        }

        // 3. Update field mode pada CPSR
        self.cpsr.set_mode(new_mode);
    }

    /// Ambil nilai register dari sudut pandang User mode (digunakan oleh LDM/STM dengan bit S)
    pub fn get_user(&self, reg: usize) -> u32 {
        let mode = self.cpsr.mode();
        if mode == CpuMode::User || mode == CpuMode::System {
            self.r[reg]
        } else {
            match reg {
                0..=7 => self.r[reg],
                8..=12 => {
                    if mode == CpuMode::Fiq {
                        self.r8_12_usr[reg - 8]
                    } else {
                        self.r[reg]
                    }
                }
                REG_SP => self.r13_bank[CpuMode::User.bank_index()],
                REG_LR => self.r14_bank[CpuMode::User.bank_index()],
                REG_PC => self.r[REG_PC],
                _ => 0,
            }
        }
    }

    /// Tulis nilai register ke User mode bank (digunakan oleh LDM/STM dengan bit S)
    pub fn set_user(&mut self, reg: usize, val: u32) {
        let mode = self.cpsr.mode();
        if mode == CpuMode::User || mode == CpuMode::System {
            self.r[reg] = val;
        } else {
            match reg {
                0..=7 => self.r[reg] = val,
                8..=12 => {
                    if mode == CpuMode::Fiq {
                        self.r8_12_usr[reg - 8] = val;
                    } else {
                        self.r[reg] = val;
                    }
                }
                REG_SP => self.r13_bank[CpuMode::User.bank_index()] = val,
                REG_LR => self.r14_bank[CpuMode::User.bank_index()] = val,
                REG_PC => self.r[REG_PC] = val,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_banking_mode_switch() {
        let mut regs = Registers::new();
        regs.change_mode(CpuMode::System);
        regs.set(REG_SP, 0x0300_7F00);
        regs.set(REG_LR, 0x0800_0100);

        // Switch to IRQ mode
        regs.change_mode(CpuMode::Irq);
        assert_eq!(regs.get(REG_SP), 0x0300_7FA0); // Pre-set in reset()
        regs.set(REG_LR, 0x0800_0500);

        // Switch back to System mode
        regs.change_mode(CpuMode::System);
        assert_eq!(regs.get(REG_SP), 0x0300_7F00);
        assert_eq!(regs.get(REG_LR), 0x0800_0100);

        // Switch back to IRQ mode
        regs.change_mode(CpuMode::Irq);
        assert_eq!(regs.get(REG_LR), 0x0800_0500);
    }

    #[test]
    fn test_fiq_r8_r12_banking() {
        let mut regs = Registers::new();
        regs.change_mode(CpuMode::System);
        regs.set(8, 0xAAAA_AAAA);
        regs.set(12, 0xBBBB_BBBB);

        regs.change_mode(CpuMode::Fiq);
        assert_eq!(regs.get(8), 0);
        regs.set(8, 0x1111_1111);

        regs.change_mode(CpuMode::System);
        assert_eq!(regs.get(8), 0xAAAA_AAAA);
        assert_eq!(regs.get(12), 0xBBBB_BBBB);

        regs.change_mode(CpuMode::Fiq);
        assert_eq!(regs.get(8), 0x1111_1111);
    }
}
