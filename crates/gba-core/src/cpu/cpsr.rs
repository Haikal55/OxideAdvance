/// Bitmask untuk flag CPSR (Current Program Status Register)
pub const FLAG_N: u32 = 1 << 31; // Negative / Less than
pub const FLAG_Z: u32 = 1 << 30; // Zero
pub const FLAG_C: u32 = 1 << 29; // Carry / Borrow
pub const FLAG_V: u32 = 1 << 28; // Overflow
pub const FLAG_I: u32 = 1 << 7;  // IRQ Disable (1 = Disable)
pub const FLAG_F: u32 = 1 << 6;  // FIQ Disable (1 = Disable)
pub const FLAG_T: u32 = 1 << 5;  // State (0 = ARM 32-bit, 1 = THUMB 16-bit)
pub const MODE_MASK: u32 = 0x1F; // Bit mode M[4:0]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuMode {
    User = 0x10,
    Fiq = 0x11,
    Irq = 0x12,
    Supervisor = 0x13,
    Abort = 0x17,
    Undefined = 0x1B,
    System = 0x1F,
}

impl CpuMode {
    pub fn from_bits(bits: u32) -> Option<Self> {
        match bits & MODE_MASK {
            0x10 => Some(Self::User),
            0x11 => Some(Self::Fiq),
            0x12 => Some(Self::Irq),
            0x13 => Some(Self::Supervisor),
            0x17 => Some(Self::Abort),
            0x1B => Some(Self::Undefined),
            0x1F => Some(Self::System),
            _ => None,
        }
    }

    /// Index untuk mapping banked register SP, LR, dan SPSR
    pub fn bank_index(self) -> usize {
        match self {
            Self::User | Self::System => 0,
            Self::Fiq => 1,
            Self::Irq => 2,
            Self::Supervisor => 3,
            Self::Abort => 4,
            Self::Undefined => 5,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cpsr {
    pub value: u32,
}

impl Cpsr {
    pub fn new(value: u32) -> Self {
        Self { value }
    }

    #[inline(always)]
    pub fn n(&self) -> bool {
        (self.value & FLAG_N) != 0
    }

    #[inline(always)]
    pub fn set_n(&mut self, val: bool) {
        if val { self.value |= FLAG_N; } else { self.value &= !FLAG_N; }
    }

    #[inline(always)]
    pub fn z(&self) -> bool {
        (self.value & FLAG_Z) != 0
    }

    #[inline(always)]
    pub fn set_z(&mut self, val: bool) {
        if val { self.value |= FLAG_Z; } else { self.value &= !FLAG_Z; }
    }

    #[inline(always)]
    pub fn c(&self) -> bool {
        (self.value & FLAG_C) != 0
    }

    #[inline(always)]
    pub fn set_c(&mut self, val: bool) {
        if val { self.value |= FLAG_C; } else { self.value &= !FLAG_C; }
    }

    #[inline(always)]
    pub fn v(&self) -> bool {
        (self.value & FLAG_V) != 0
    }

    #[inline(always)]
    pub fn set_v(&mut self, val: bool) {
        if val { self.value |= FLAG_V; } else { self.value &= !FLAG_V; }
    }

    #[inline(always)]
    pub fn irq_disabled(&self) -> bool {
        (self.value & FLAG_I) != 0
    }

    #[inline(always)]
    pub fn set_irq_disabled(&mut self, val: bool) {
        if val { self.value |= FLAG_I; } else { self.value &= !FLAG_I; }
    }

    #[inline(always)]
    pub fn fiq_disabled(&self) -> bool {
        (self.value & FLAG_F) != 0
    }

    #[inline(always)]
    pub fn set_fiq_disabled(&mut self, val: bool) {
        if val { self.value |= FLAG_F; } else { self.value &= !FLAG_F; }
    }

    #[inline(always)]
    pub fn is_thumb(&self) -> bool {
        (self.value & FLAG_T) != 0
    }

    #[inline(always)]
    pub fn set_thumb(&mut self, val: bool) {
        if val { self.value |= FLAG_T; } else { self.value &= !FLAG_T; }
    }

    #[inline(always)]
    pub fn mode(&self) -> CpuMode {
        CpuMode::from_bits(self.value).unwrap_or(CpuMode::System)
    }

    pub fn set_mode(&mut self, mode: CpuMode) {
        self.value = (self.value & !MODE_MASK) | (mode as u32);
    }

    /// Evaluasi apakah kondisi 4-bit pada instruksi ARM terpenuhi berdasarkan flag CPSR
    #[inline(always)]
    pub fn satisfies_condition(&self, cond: u32) -> bool {
        match cond {
            0x0 => self.z(),                       // EQ (Equal)
            0x1 => !self.z(),                      // NE (Not equal)
            0x2 => self.c(),                       // CS / HS (Carry set / Unsigned higher or same)
            0x3 => !self.c(),                      // CC / LO (Carry clear / Unsigned lower)
            0x4 => self.n(),                       // MI (Minus / Negative)
            0x5 => !self.n(),                      // PL (Plus / Positive or zero)
            0x6 => self.v(),                       // VS (Overflow set)
            0x7 => !self.v(),                      // VC (Overflow clear)
            0x8 => self.c() && !self.z(),          // HI (Unsigned higher)
            0x9 => !self.c() || self.z(),          // LS (Unsigned lower or same)
            0xA => self.n() == self.v(),           // GE (Signed greater than or equal)
            0xB => self.n() != self.v(),           // LT (Signed less than)
            0xC => !self.z() && (self.n() == self.v()), // GT (Signed greater than)
            0xD => self.z() || (self.n() != self.v()),  // LE (Signed less than or equal)
            0xE => true,                           // AL (Always)
            0xF => false,                          // NV (Never - Reserved di ARMv4)
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpsr_flags_and_conditions() {
        let mut cpsr = Cpsr::new(0);
        cpsr.set_z(true);
        assert!(cpsr.satisfies_condition(0x0)); // EQ
        assert!(!cpsr.satisfies_condition(0x1)); // NE

        cpsr.set_z(false);
        cpsr.set_c(true);
        assert!(cpsr.satisfies_condition(0x2)); // CS / HS
        assert!(!cpsr.satisfies_condition(0x3)); // CC / LO
        assert!(cpsr.satisfies_condition(0x8)); // HI (C=1 and Z=0)

        // GE: N == V
        cpsr.set_n(true);
        cpsr.set_v(true);
        assert!(cpsr.satisfies_condition(0xA)); // GE (true == true)
        cpsr.set_v(false);
        assert!(!cpsr.satisfies_condition(0xA)); // GE (true != false)
    }

    #[test]
    fn test_cpu_mode_parsing() {
        assert_eq!(CpuMode::from_bits(0x10), Some(CpuMode::User));
        assert_eq!(CpuMode::from_bits(0x12), Some(CpuMode::Irq));
        assert_eq!(CpuMode::from_bits(0x1F), Some(CpuMode::System));
    }
}
