/// Jenis pergeseran bit (Shift Type) pada Barrel Shifter ARM
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftType {
    Lsl = 0, // Logical Shift Left
    Lsr = 1, // Logical Shift Right
    Asr = 2, // Arithmetic Shift Right
    Ror = 3, // Rotate Right / Rotate Right with Extend (RRX)
}

impl ShiftType {
    #[inline(always)]
    pub fn from_bits(bits: u32) -> Self {
        match bits & 0x3 {
            0 => Self::Lsl,
            1 => Self::Lsr,
            2 => Self::Asr,
            3 => Self::Ror,
            _ => unreachable!(),
        }
    }
}

/// Operasi ALU 4-bit (bits 24..=21 pada instruksi Data Processing ARM)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AluOp {
    And = 0x0,
    Eor = 0x1,
    Sub = 0x2,
    Rsb = 0x3,
    Add = 0x4,
    Adc = 0x5,
    Sbc = 0x6,
    Rsc = 0x7,
    Tst = 0x8,
    Teq = 0x9,
    Cmp = 0xA,
    Cmn = 0xB,
    Orr = 0xC,
    Mov = 0xD,
    Bic = 0xE,
    Mvn = 0xF,
}

impl AluOp {
    #[inline(always)]
    pub fn from_bits(bits: u32) -> Self {
        match bits & 0xF {
            0x0 => Self::And,
            0x1 => Self::Eor,
            0x2 => Self::Sub,
            0x3 => Self::Rsb,
            0x4 => Self::Add,
            0x5 => Self::Adc,
            0x6 => Self::Sbc,
            0x7 => Self::Rsc,
            0x8 => Self::Tst,
            0x9 => Self::Teq,
            0xA => Self::Cmp,
            0xB => Self::Cmn,
            0xC => Self::Orr,
            0xD => Self::Mov,
            0xE => Self::Bic,
            0xF => Self::Mvn,
            _ => unreachable!(),
        }
    }

    /// Apakah operasi merupakan instruksi test (hanya update flag, tidak menulis ke Rd)
    #[inline(always)]
    pub fn is_test(self) -> bool {
        matches!(self, Self::Tst | Self::Teq | Self::Cmp | Self::Cmn)
    }

    /// Apakah operasi merupakan operasi aritmatika (menghasilkan Carry & Overflow flag)
    #[inline(always)]
    pub fn is_arithmetic(self) -> bool {
        matches!(
            self,
            Self::Sub | Self::Rsb | Self::Add | Self::Adc | Self::Sbc | Self::Rsc | Self::Cmp | Self::Cmn
        )
    }
}

/// Hasil komputasi ALU
#[derive(Clone, Copy, Debug)]
pub struct AluOutput {
    pub result: u32,
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: Option<bool>, // None untuk operasi logika (flag V tidak berubah)
}

/// Barrel Shifter: Pergeseran menggunakan nilai Immediate (5-bit shift amount)
pub fn shift_by_imm(val: u32, shift_type: ShiftType, imm: u32, carry_in: bool) -> (u32, bool) {
    match shift_type {
        ShiftType::Lsl => {
            if imm == 0 {
                // LSL #0: Nilai tidak bergeser, Carry out tetap bernilai carry_in
                (val, carry_in)
            } else {
                let carry_out = ((val >> (32 - imm)) & 1) != 0;
                (val << imm, carry_out)
            }
        }
        ShiftType::Lsr => {
            if imm == 0 {
                // LSR #0 pada hardware di-encode sebagai LSR #32
                (0, (val & 0x8000_0000) != 0)
            } else {
                let carry_out = ((val >> (imm - 1)) & 1) != 0;
                (val >> imm, carry_out)
            }
        }
        ShiftType::Asr => {
            if imm == 0 {
                // ASR #0 di-encode sebagai ASR #32
                if (val & 0x8000_0000) != 0 {
                    (0xFFFF_FFFF, true)
                } else {
                    (0, false)
                }
            } else {
                let carry_out = ((val >> (imm - 1)) & 1) != 0;
                (((val as i32) >> imm) as u32, carry_out)
            }
        }
        ShiftType::Ror => {
            if imm == 0 {
                // ROR #0 di-encode sebagai RRX (Rotate Right with Extend through Carry)
                let carry_out = (val & 1) != 0;
                let res = ((carry_in as u32) << 31) | (val >> 1);
                (res, carry_out)
            } else {
                let carry_out = ((val >> (imm - 1)) & 1) != 0;
                (val.rotate_right(imm), carry_out)
            }
        }
    }
}

/// Barrel Shifter: Pergeseran menggunakan nilai Register (hanya byte bawah Rs[7:0] yang digunakan)
pub fn shift_by_reg(val: u32, shift_type: ShiftType, shift_reg: u32, carry_in: bool) -> (u32, bool) {
    let amt = (shift_reg & 0xFF) as usize;

    if amt == 0 {
        return (val, carry_in);
    }

    match shift_type {
        ShiftType::Lsl => {
            if amt < 32 {
                let carry_out = ((val >> (32 - amt)) & 1) != 0;
                (val << amt, carry_out)
            } else if amt == 32 {
                (0, (val & 1) != 0)
            } else {
                (0, false)
            }
        }
        ShiftType::Lsr => {
            if amt < 32 {
                let carry_out = ((val >> (amt - 1)) & 1) != 0;
                (val >> amt, carry_out)
            } else if amt == 32 {
                (0, (val & 0x8000_0000) != 0)
            } else {
                (0, false)
            }
        }
        ShiftType::Asr => {
            if amt < 32 {
                let carry_out = ((val >> (amt - 1)) & 1) != 0;
                (((val as i32) >> amt) as u32, carry_out)
            } else if (val & 0x8000_0000) != 0 {
                (0xFFFF_FFFF, true)
            } else {
                (0, false)
            }
        }
        ShiftType::Ror => {
            let amt_mod = amt % 32;
            if amt_mod == 0 {
                (val, (val & 0x8000_0000) != 0)
            } else {
                let carry_out = ((val >> (amt_mod - 1)) & 1) != 0;
                (val.rotate_right(amt_mod as u32), carry_out)
            }
        }
    }
}

/// Eksekusi Operasi ALU dan kalkulasi flag N, Z, C, V
pub fn execute_alu(
    op: AluOp,
    a: u32,
    b: u32,
    carry_in: bool,
    shifter_carry: bool,
) -> AluOutput {
    let mut v = None;

    let (result, c) = match op {
        AluOp::And | AluOp::Tst => (a & b, shifter_carry),
        AluOp::Eor | AluOp::Teq => (a ^ b, shifter_carry),
        AluOp::Orr => (a | b, shifter_carry),
        AluOp::Mov => (b, shifter_carry),
        AluOp::Bic => (a & !b, shifter_carry),
        AluOp::Mvn => (!b, shifter_carry),

        AluOp::Add | AluOp::Cmn => {
            let sum = (a as u64) + (b as u64);
            let res = sum as u32;
            let overflow = (!(a ^ b) & (a ^ res) & 0x8000_0000) != 0;
            v = Some(overflow);
            (res, sum > 0xFFFF_FFFF)
        }

        AluOp::Adc => {
            let sum = (a as u64) + (b as u64) + (carry_in as u64);
            let res = sum as u32;
            let overflow = (!(a ^ b) & (a ^ res) & 0x8000_0000) != 0;
            v = Some(overflow);
            (res, sum > 0xFFFF_FFFF)
        }

        AluOp::Sub | AluOp::Cmp => {
            let res = a.wrapping_sub(b);
            let carry = a >= b; // Not borrow pada ARM
            let overflow = ((a ^ b) & (a ^ res) & 0x8000_0000) != 0;
            v = Some(overflow);
            (res, carry)
        }

        AluOp::Rsb => {
            let res = b.wrapping_sub(a);
            let carry = b >= a; // Not borrow
            let overflow = ((b ^ a) & (b ^ res) & 0x8000_0000) != 0;
            v = Some(overflow);
            (res, carry)
        }

        AluOp::Sbc => {
            // Pada ARM: a - b - NOT(carry_in) = a + NOT(b) + carry_in
            let sum = (a as u64) + (!b as u32 as u64) + (carry_in as u64);
            let res = a.wrapping_sub(b).wrapping_sub((!carry_in) as u32);
            let carry = sum > 0xFFFF_FFFF;
            let overflow = ((a ^ b) & (a ^ res) & 0x8000_0000) != 0;
            v = Some(overflow);
            (res, carry)
        }

        AluOp::Rsc => {
            // Pada ARM: b - a - NOT(carry_in) = b + NOT(a) + carry_in
            let sum = (b as u64) + (!a as u32 as u64) + (carry_in as u64);
            let res = b.wrapping_sub(a).wrapping_sub((!carry_in) as u32);
            let carry = sum > 0xFFFF_FFFF;
            let overflow = ((b ^ a) & (b ^ res) & 0x8000_0000) != 0;
            v = Some(overflow);
            (res, carry)
        }
    };

    let n = (result & 0x8000_0000) != 0;
    let z = result == 0;

    AluOutput {
        result,
        n,
        z,
        c,
        v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_adc() {
        // Simple ADD without carry
        let out = execute_alu(AluOp::Add, 10, 20, false, false);
        assert_eq!(out.result, 30);
        assert!(!out.c);
        assert_eq!(out.v, Some(false));
        assert!(!out.n);
        assert!(!out.z);

        // ADD with unsigned carry
        let out = execute_alu(AluOp::Add, 0xFFFF_FFFF, 1, false, false);
        assert_eq!(out.result, 0);
        assert!(out.c);
        assert!(out.z);
        assert_eq!(out.v, Some(false));

        // ADD with signed overflow (positive + positive = negative)
        let out = execute_alu(AluOp::Add, 0x7FFF_FFFF, 1, false, false);
        assert_eq!(out.result, 0x8000_0000);
        assert!(!out.c);
        assert!(out.n);
        assert_eq!(out.v, Some(true));

        // ADC with carry_in = true
        let out = execute_alu(AluOp::Adc, 10, 20, true, false);
        assert_eq!(out.result, 31);
    }

    #[test]
    fn test_sub_and_sbc() {
        // SUB without borrow (Carry flag = 1)
        let out = execute_alu(AluOp::Sub, 20, 10, false, false);
        assert_eq!(out.result, 10);
        assert!(out.c); // ARM: Carry set when no borrow
        assert_eq!(out.v, Some(false));

        // SUB with borrow (Carry flag = 0)
        let out = execute_alu(AluOp::Sub, 10, 20, false, false);
        assert_eq!(out.result, (-10i32) as u32);
        assert!(!out.c); // Borrow occurred
        assert!(out.n);

        // SUB with signed overflow: negative - positive = positive
        let out = execute_alu(AluOp::Sub, 0x8000_0000, 1, false, false);
        assert_eq!(out.result, 0x7FFF_FFFF);
        assert!(out.c);
        assert_eq!(out.v, Some(true));

        // SBC with carry_in = 1 (meaning no previous borrow)
        let out = execute_alu(AluOp::Sbc, 20, 10, true, false);
        assert_eq!(out.result, 10);
        assert!(out.c);

        // SBC with carry_in = 0 (meaning previous borrow)
        let out = execute_alu(AluOp::Sbc, 20, 10, false, false);
        assert_eq!(out.result, 9);
        assert!(out.c);
    }

    #[test]
    fn test_barrel_shifter_imm() {
        // LSL #2
        let (res, c) = shift_by_imm(0b1011, ShiftType::Lsl, 2, false);
        assert_eq!(res, 0b101100);
        assert!(!c);

        // LSR #1
        let (res, c) = shift_by_imm(0b1011, ShiftType::Lsr, 1, false);
        assert_eq!(res, 0b101);
        assert!(c); // Discarded bit 0 was 1

        // LSR #0 (represents LSR #32)
        let (res, c) = shift_by_imm(0x8000_0000, ShiftType::Lsr, 0, false);
        assert_eq!(res, 0);
        assert!(c);

        // ASR #2 with negative number
        let (res, c) = shift_by_imm(0x8000_0004, ShiftType::Asr, 2, false);
        assert_eq!(res, 0xE000_0001);
        assert!(!c);

        // RRX (ROR #0) with carry_in = true
        let (res, c) = shift_by_imm(0x0000_0001, ShiftType::Ror, 0, true);
        assert_eq!(res, 0x8000_0000);
        assert!(c); // Bit 0 was 1
    }

    #[test]
    fn test_barrel_shifter_reg() {
        // Shift amount 0 preserves carry
        let (res, c) = shift_by_reg(0x1234, ShiftType::Lsl, 0, true);
        assert_eq!(res, 0x1234);
        assert!(c);

        // LSL #32
        let (res, c) = shift_by_reg(0x0000_0001, ShiftType::Lsl, 32, false);
        assert_eq!(res, 0);
        assert!(c);

        // LSL > 32
        let (res, c) = shift_by_reg(0x0000_0001, ShiftType::Lsl, 33, false);
        assert_eq!(res, 0);
        assert!(!c);
    }
}
