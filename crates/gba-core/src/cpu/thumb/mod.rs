use crate::bus::Bus;
use crate::cpu::alu::{execute_alu, shift_by_imm, shift_by_reg, AluOp, ShiftType};
use crate::cpu::cpsr::CpuMode;
use crate::cpu::registers::{REG_LR, REG_PC, REG_SP};
use crate::cpu::Arm7Tdmi;

/// Menangani dekode dan eksekusi instruksi 16-bit THUMB
pub fn execute_thumb(cpu: &mut Arm7Tdmi, bus: &mut impl Bus, instruction: u16) -> usize {
    let instr = instruction as u32;

    // Format 1: Move shifted register (bits [15:11] == 000xx, kecuali 00011 yang merupakan Format 2)
    if (instr & 0xE000) == 0x0000 && (instr & 0x1800) != 0x1800 {
        let op = (instr >> 11) & 0x3;
        let imm5 = (instr >> 6) & 0x1F;
        let rs = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;

        let shift_type = match op {
            0 => ShiftType::Lsl,
            1 => ShiftType::Lsr,
            2 => ShiftType::Asr,
            _ => unreachable!(),
        };

        let val = cpu.regs.get(rs);
        let (res, carry) = shift_by_imm(val, shift_type, imm5, cpu.regs.cpsr.c());
        cpu.regs.set(rd, res);

        cpu.regs.cpsr.set_n((res & 0x8000_0000) != 0);
        cpu.regs.cpsr.set_z(res == 0);
        cpu.regs.cpsr.set_c(carry);
        return 1;
    }

    // Format 2: Add / Subtract (bits [15:11] == 00011)
    if (instr & 0xF800) == 0x1800 {
        let is_imm = (instr & (1 << 10)) != 0;
        let is_sub = (instr & (1 << 9)) != 0;
        let rn_imm3 = (instr >> 6) & 0x7;
        let rs = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;

        let op1 = cpu.regs.get(rs);
        let op2 = if is_imm {
            rn_imm3
        } else {
            cpu.regs.get(rn_imm3 as usize)
        };

        let alu_op = if is_sub { AluOp::Sub } else { AluOp::Add };
        let out = execute_alu(alu_op, op1, op2, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());

        cpu.regs.set(rd, out.result);
        cpu.regs.cpsr.set_n(out.n);
        cpu.regs.cpsr.set_z(out.z);
        cpu.regs.cpsr.set_c(out.c);
        if let Some(v) = out.v {
            cpu.regs.cpsr.set_v(v);
        }
        return 1;
    }

    // Format 3: Move / Compare / Add / Subtract Immediate (bits [15:13] == 001)
    if (instr & 0xE000) == 0x2000 {
        let op = (instr >> 11) & 0x3;
        let rd = ((instr >> 8) & 0x7) as usize;
        let imm8 = instr & 0xFF;

        let op1 = cpu.regs.get(rd);

        match op {
            0 => {
                // MOV Rd, #imm8
                cpu.regs.set(rd, imm8);
                cpu.regs.cpsr.set_n(false);
                cpu.regs.cpsr.set_z(imm8 == 0);
            }
            1 => {
                // CMP Rd, #imm8
                let out = execute_alu(AluOp::Cmp, op1, imm8, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
                cpu.regs.cpsr.set_c(out.c);
                if let Some(v) = out.v {
                    cpu.regs.cpsr.set_v(v);
                }
            }
            2 => {
                // ADD Rd, #imm8
                let out = execute_alu(AluOp::Add, op1, imm8, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.set(rd, out.result);
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
                cpu.regs.cpsr.set_c(out.c);
                if let Some(v) = out.v {
                    cpu.regs.cpsr.set_v(v);
                }
            }
            3 => {
                // SUB Rd, #imm8
                let out = execute_alu(AluOp::Sub, op1, imm8, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.set(rd, out.result);
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
                cpu.regs.cpsr.set_c(out.c);
                if let Some(v) = out.v {
                    cpu.regs.cpsr.set_v(v);
                }
            }
            _ => unreachable!(),
        }
        return 1;
    }

    // Format 4: ALU Operations (bits [15:10] == 010000)
    if (instr & 0xFC00) == 0x4000 {
        let op = (instr >> 6) & 0xF;
        let rs = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;

        let op1 = cpu.regs.get(rd);
        let op2 = cpu.regs.get(rs);

        match op {
            0 => {
                // AND
                let out = execute_alu(AluOp::And, op1, op2, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.set(rd, out.result);
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
            }
            1 => {
                // EOR
                let out = execute_alu(AluOp::Eor, op1, op2, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.set(rd, out.result);
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
            }
            2 => {
                // LSL
                let (res, c) = shift_by_reg(op1, ShiftType::Lsl, op2, cpu.regs.cpsr.c());
                cpu.regs.set(rd, res);
                cpu.regs.cpsr.set_n((res & 0x8000_0000) != 0);
                cpu.regs.cpsr.set_z(res == 0);
                cpu.regs.cpsr.set_c(c);
            }
            3 => {
                // LSR
                let (res, c) = shift_by_reg(op1, ShiftType::Lsr, op2, cpu.regs.cpsr.c());
                cpu.regs.set(rd, res);
                cpu.regs.cpsr.set_n((res & 0x8000_0000) != 0);
                cpu.regs.cpsr.set_z(res == 0);
                cpu.regs.cpsr.set_c(c);
            }
            4 => {
                // ASR
                let (res, c) = shift_by_reg(op1, ShiftType::Asr, op2, cpu.regs.cpsr.c());
                cpu.regs.set(rd, res);
                cpu.regs.cpsr.set_n((res & 0x8000_0000) != 0);
                cpu.regs.cpsr.set_z(res == 0);
                cpu.regs.cpsr.set_c(c);
            }
            5 => {
                // ADC
                let out = execute_alu(AluOp::Adc, op1, op2, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.set(rd, out.result);
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
                cpu.regs.cpsr.set_c(out.c);
                if let Some(v) = out.v {
                    cpu.regs.cpsr.set_v(v);
                }
            }
            6 => {
                // SBC
                let out = execute_alu(AluOp::Sbc, op1, op2, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.set(rd, out.result);
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
                cpu.regs.cpsr.set_c(out.c);
                if let Some(v) = out.v {
                    cpu.regs.cpsr.set_v(v);
                }
            }
            7 => {
                // ROR
                let (res, c) = shift_by_reg(op1, ShiftType::Ror, op2, cpu.regs.cpsr.c());
                cpu.regs.set(rd, res);
                cpu.regs.cpsr.set_n((res & 0x8000_0000) != 0);
                cpu.regs.cpsr.set_z(res == 0);
                cpu.regs.cpsr.set_c(c);
            }
            8 => {
                // TST
                let out = execute_alu(AluOp::Tst, op1, op2, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
            }
            9 => {
                // NEG (0 - Rs)
                let out = execute_alu(AluOp::Sub, 0, op2, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.set(rd, out.result);
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
                cpu.regs.cpsr.set_c(out.c);
                if let Some(v) = out.v {
                    cpu.regs.cpsr.set_v(v);
                }
            }
            10 => {
                // CMP
                let out = execute_alu(AluOp::Cmp, op1, op2, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
                cpu.regs.cpsr.set_c(out.c);
                if let Some(v) = out.v {
                    cpu.regs.cpsr.set_v(v);
                }
            }
            11 => {
                // CMN
                let out = execute_alu(AluOp::Cmn, op1, op2, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
                cpu.regs.cpsr.set_c(out.c);
                if let Some(v) = out.v {
                    cpu.regs.cpsr.set_v(v);
                }
            }
            12 => {
                // ORR
                let out = execute_alu(AluOp::Orr, op1, op2, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.set(rd, out.result);
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
            }
            13 => {
                // MUL (Rd = Rs * Rd)
                let res = op1.wrapping_mul(op2);
                cpu.regs.set(rd, res);
                cpu.regs.cpsr.set_n((res & 0x8000_0000) != 0);
                cpu.regs.cpsr.set_z(res == 0);
            }
            14 => {
                // BIC (Rd & !Rs)
                let out = execute_alu(AluOp::Bic, op1, op2, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.set(rd, out.result);
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
            }
            15 => {
                // MVN (!Rs)
                let out = execute_alu(AluOp::Mvn, op1, op2, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.set(rd, out.result);
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
            }
            _ => unreachable!(),
        }
        return 1;
    }

    // Format 5: Hi Register Operations / Branch Exchange (bits [15:10] == 010001)
    if (instr & 0xFC00) == 0x4400 {
        let op = (instr >> 8) & 0x3;
        let h1 = ((instr >> 7) & 1) as usize;
        let h2 = ((instr >> 6) & 1) as usize;
        let rs = (h2 << 3) | ((instr >> 3) & 0x7) as usize;
        let rd = (h1 << 3) | (instr & 0x7) as usize;

        let op1 = cpu.regs.get(rd);
        let op2 = cpu.regs.get(rs);

        match op {
            0 => {
                // ADD Rd, Rs
                let res = op1.wrapping_add(op2);
                cpu.regs.set(rd, res);
                if rd == REG_PC {
                    cpu.refill_pipeline(bus);
                    return 3;
                }
            }
            1 => {
                // CMP Rd, Rs
                let out = execute_alu(AluOp::Cmp, op1, op2, cpu.regs.cpsr.c(), cpu.regs.cpsr.c());
                cpu.regs.cpsr.set_n(out.n);
                cpu.regs.cpsr.set_z(out.z);
                cpu.regs.cpsr.set_c(out.c);
                if let Some(v) = out.v {
                    cpu.regs.cpsr.set_v(v);
                }
            }
            2 => {
                // MOV Rd, Rs
                cpu.regs.set(rd, op2);
                if rd == REG_PC {
                    cpu.refill_pipeline(bus);
                    return 3;
                }
            }
            3 => {
                // BX Rs
                let target = op2;
                let is_thumb = (target & 1) != 0;
                cpu.regs.cpsr.set_thumb(is_thumb);

                let target_pc = if is_thumb {
                    target & !1
                } else {
                    target & !3
                };
                cpu.regs.set_pc(target_pc);
                cpu.refill_pipeline(bus);
                return 3;
            }
            _ => unreachable!(),
        }
        return 1;
    }

    // Format 6: PC-relative Load: LDR Rd, [PC, #imm8 * 4] (bits [15:11] == 01001)
    if (instr & 0xF800) == 0x4800 {
        let rd = ((instr >> 8) & 0x7) as usize;
        let imm8 = (instr & 0xFF) * 4;
        // Alamat PC di-align ke kelipatan 4 bytes pada THUMB Format 6
        let addr = (cpu.regs.pc() & !0x2).wrapping_add(imm8);
        let val = bus.read32(addr & !3);
        cpu.regs.set(rd, val);
        return 2;
    }

    // Format 7: Load/Store with register offset (bits [15:12] == 0101, bit 9 == 0)
    if (instr & 0xF200) == 0x5000 {
        let is_load = (instr & (1 << 11)) != 0;
        let is_byte = (instr & (1 << 10)) != 0;
        let ro = ((instr >> 6) & 0x7) as usize;
        let rb = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;

        let addr = cpu.regs.get(rb).wrapping_add(cpu.regs.get(ro));

        if is_load {
            let val = if is_byte {
                bus.read8(addr) as u32
            } else {
                bus.read32_unaligned(addr)
            };
            cpu.regs.set(rd, val);
            return 2;
        } else {
            let val = cpu.regs.get(rd);
            if is_byte {
                bus.write8(addr, val as u8);
            } else {
                bus.write32(addr & !3, val);
            }
            return 2;
        }
    }

    // Format 8: Load/Store sign-extended byte/halfword (bits [15:12] == 0101, bit 9 == 1)
    if (instr & 0xF200) == 0x5200 {
        let h = (instr & (1 << 11)) != 0;
        let s = (instr & (1 << 10)) != 0;
        let ro = ((instr >> 6) & 0x7) as usize;
        let rb = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;

        let addr = cpu.regs.get(rb).wrapping_add(cpu.regs.get(ro));

        match (h, s) {
            (false, false) => {
                // STRH
                bus.write16(addr & !1, cpu.regs.get(rd) as u16);
            }
            (false, true) => {
                // LDRSB
                let val = (bus.read8(addr) as i8 as i32) as u32;
                cpu.regs.set(rd, val);
            }
            (true, false) => {
                // LDRH
                let val = bus.read16_unaligned(addr) as u32;
                cpu.regs.set(rd, val);
            }
            (true, true) => {
                // LDRSH
                let val = if (addr & 1) == 0 {
                    (bus.read16(addr) as i16 as i32) as u32
                } else {
                    (bus.read8(addr) as i8 as i32) as u32
                };
                cpu.regs.set(rd, val);
            }
        }
        return 2;
    }

    // Format 9: Load/Store with immediate offset (bits [15:13] == 011)
    if (instr & 0xE000) == 0x6000 {
        let is_byte = (instr & (1 << 12)) != 0;
        let is_load = (instr & (1 << 11)) != 0;
        let imm5 = (instr >> 6) & 0x1F;
        let rb = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;

        let offset = if is_byte { imm5 } else { imm5 * 4 };
        let addr = cpu.regs.get(rb).wrapping_add(offset);

        if is_load {
            let val = if is_byte {
                bus.read8(addr) as u32
            } else {
                bus.read32_unaligned(addr)
            };
            cpu.regs.set(rd, val);
            return 2;
        } else {
            let val = cpu.regs.get(rd);
            if is_byte {
                bus.write8(addr, val as u8);
            } else {
                bus.write32(addr & !3, val);
            }
            return 2;
        }
    }

    // Format 10: Load/Store Halfword immediate offset (bits [15:12] == 1000)
    if (instr & 0xF000) == 0x8000 {
        let is_load = (instr & (1 << 11)) != 0;
        let offset = ((instr >> 6) & 0x1F) * 2;
        let rb = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;

        let addr = cpu.regs.get(rb).wrapping_add(offset);

        if is_load {
            let val = bus.read16_unaligned(addr) as u32;
            cpu.regs.set(rd, val);
        } else {
            bus.write16(addr & !1, cpu.regs.get(rd) as u16);
        }
        return 2;
    }

    // Format 11: SP-relative Load/Store: LDR/STR Rd, [SP, #imm8 * 4] (bits [15:12] == 1001)
    if (instr & 0xF000) == 0x9000 {
        let is_load = (instr & (1 << 11)) != 0;
        let rd = ((instr >> 8) & 0x7) as usize;
        let offset = (instr & 0xFF) * 4;

        let addr = cpu.regs.get(REG_SP).wrapping_add(offset);

        if is_load {
            let val = bus.read32_unaligned(addr);
            cpu.regs.set(rd, val);
        } else {
            bus.write32(addr & !3, cpu.regs.get(rd));
        }
        return 2;
    }

    // Format 12: Load Address: ADD Rd, PC/SP, #imm8 * 4 (bits [15:12] == 1010)
    if (instr & 0xF000) == 0xA000 {
        let is_sp = (instr & (1 << 11)) != 0;
        let rd = ((instr >> 8) & 0x7) as usize;
        let offset = (instr & 0xFF) * 4;

        let base = if is_sp {
            cpu.regs.get(REG_SP)
        } else {
            cpu.regs.pc() & !0x2
        };

        cpu.regs.set(rd, base.wrapping_add(offset));
        return 1;
    }

    // Format 13: Add offset to Stack Pointer: ADD/SUB SP, #imm7 * 4 (bits [15:8] == 10110000)
    if (instr & 0xFF00) == 0xB000 {
        let is_negative = (instr & (1 << 7)) != 0;
        let offset = (instr & 0x7F) * 4;
        let sp = cpu.regs.get(REG_SP);

        let new_sp = if is_negative {
            sp.wrapping_sub(offset)
        } else {
            sp.wrapping_add(offset)
        };

        cpu.regs.set(REG_SP, new_sp);
        return 1;
    }

    // Format 14: Push / Pop Registers (bits [15:12] == 1011, bits [11:10] == 10)
    if (instr & 0xF600) == 0xB400 {
        let is_pop = (instr & (1 << 11)) != 0;
        let r_bit = (instr & (1 << 8)) != 0;
        let rlist = instr & 0xFF;

        let count = rlist.count_ones() + if r_bit { 1 } else { 0 };

        if is_pop {
            // POP: muat dari SP dan naikkan SP
            let mut addr = cpu.regs.get(REG_SP);

            for i in 0..8 {
                if (rlist & (1 << i)) != 0 {
                    cpu.regs.set(i, bus.read32(addr));
                    addr = addr.wrapping_add(4);
                }
            }

            if r_bit {
                let target = bus.read32(addr);
                addr = addr.wrapping_add(4);
                cpu.regs.set_pc(target & !1);
                cpu.refill_pipeline(bus);
            }

            cpu.regs.set(REG_SP, addr);
            return (count as usize) + if r_bit { 3 } else { 1 };
        } else {
            // PUSH: kurangi SP dan simpan data
            let mut addr = cpu.regs.get(REG_SP).wrapping_sub(count * 4);
            cpu.regs.set(REG_SP, addr);

            for i in 0..8 {
                if (rlist & (1 << i)) != 0 {
                    bus.write32(addr, cpu.regs.get(i));
                    addr = addr.wrapping_add(4);
                }
            }

            if r_bit {
                bus.write32(addr, cpu.regs.get(REG_LR));
            }

            return (count as usize) + 1;
        }
    }

    // Format 15: Multiple Load / Store: LDMIA/STMIA Rb!, {Rlist} (bits [15:12] == 1100)
    if (instr & 0xF000) == 0xC000 {
        let is_load = (instr & (1 << 11)) != 0;
        let rb = ((instr >> 8) & 0x7) as usize;
        let rlist = instr & 0xFF;
        let count = rlist.count_ones();

        let mut addr = cpu.regs.get(rb);

        for i in 0..8 {
            if (rlist & (1 << i)) != 0 {
                if is_load {
                    cpu.regs.set(i, bus.read32(addr));
                } else {
                    bus.write32(addr, cpu.regs.get(i));
                }
                addr = addr.wrapping_add(4);
            }
        }

        cpu.regs.set(rb, addr);
        return (count as usize) + 1;
    }

    // Format 16: Conditional Branch B<cond> (bits [15:12] == 1101, bits [11:8] < 1110)
    if (instr & 0xF000) == 0xD000 && ((instr >> 8) & 0xF) < 0xE {
        let cond = (instr >> 8) & 0xF;
        if cpu.regs.cpsr.satisfies_condition(cond) {
            let mut offset = (instr & 0xFF) as i32;
            if (offset & 0x80) != 0 {
                offset |= !0xFF;
            }
            offset <<= 1;
            let target = (cpu.regs.pc() as i32).wrapping_add(offset) as u32;
            cpu.regs.set_pc(target);
            cpu.refill_pipeline(bus);
            return 3;
        }
        return 1;
    }

    // Format 17: Software Interrupt (SWI) (bits [15:8] == 11011111)
    if (instr & 0xFF00) == 0xDF00 {
        let swi_num = (instr & 0xFF) as u8;
        if bus.is_bios_hle() {
            return crate::bios::hle::handle_swi(cpu, bus, swi_num);
        }

        let cpsr = cpu.regs.cpsr.value;
        let return_pc = cpu.regs.pc().wrapping_sub(2);

        cpu.regs.change_mode(CpuMode::Supervisor);
        cpu.regs.set_spsr(cpsr);
        cpu.regs.set(REG_LR, return_pc);

        cpu.regs.cpsr.set_thumb(false);
        cpu.regs.cpsr.set_irq_disabled(true);

        cpu.regs.set_pc(0x0000_0008);
        cpu.refill_pipeline(bus);
        return 3;
    }

    // Format 18: Unconditional branch B <label> (bits [15:11] == 11100)
    if (instr & 0xF800) == 0xE000 {
        let mut offset = (instr & 0x07FF) as i32;
        if (offset & 0x0400) != 0 {
            offset |= !0x07FF;
        }
        offset <<= 1;

        let target = (cpu.regs.pc() as i32).wrapping_add(offset) as u32;
        cpu.regs.set_pc(target);
        cpu.refill_pipeline(bus);
        return 3;
    }

    // Format 19: Long branch with link (BL)
    // BL Prefix: 11110 [Offset High 11 bits]
    if (instr & 0xF800) == 0xF000 {
        let mut offset = (instr & 0x07FF) as i32;
        if (offset & 0x0400) != 0 {
            offset |= !0x07FF;
        }
        let target = (cpu.regs.pc() as i32).wrapping_add(offset << 12) as u32;
        cpu.regs.set(REG_LR, target);
        return 1;
    }

    // BL Suffix: 11111 [Offset Low 11 bits]
    if (instr & 0xF800) == 0xF800 {
        let offset = (instr & 0x07FF) << 1;
        let target = cpu.regs.get(REG_LR).wrapping_add(offset);
        let next_pc = (cpu.regs.pc().wrapping_sub(2)) | 1;
        cpu.regs.set(REG_LR, next_pc);

        cpu.regs.set_pc(target);
        cpu.refill_pipeline(bus);
        return 3;
    }

    // Default 1 cycle untuk instruksi unhandled
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::GbaBus;

    #[test]
    fn test_thumb_mov_add_sub_format3() {
        let mut cpu = Arm7Tdmi::new();
        let mut bus = GbaBus::new();
        cpu.regs.cpsr.set_thumb(true);

        // MOV R0, #25 -> 0x2019
        execute_thumb(&mut cpu, &mut bus, 0x2019);
        assert_eq!(cpu.regs.get(0), 25);

        // ADD R0, #15 -> 0x300F
        execute_thumb(&mut cpu, &mut bus, 0x300F);
        assert_eq!(cpu.regs.get(0), 40);

        // SUB R0, #10 -> 0x380A
        execute_thumb(&mut cpu, &mut bus, 0x380A);
        assert_eq!(cpu.regs.get(0), 30);
    }

    #[test]
    fn test_thumb_alu_and_cmp_format4() {
        let mut cpu = Arm7Tdmi::new();
        let mut bus = GbaBus::new();
        cpu.regs.cpsr.set_thumb(true);

        cpu.regs.set(0, 50);
        cpu.regs.set(1, 50);

        // CMP R0, R1 -> 0x4288
        execute_thumb(&mut cpu, &mut bus, 0x4288);
        assert!(cpu.regs.cpsr.z());
        assert!(cpu.regs.cpsr.c());
    }

    #[test]
    fn test_thumb_push_pop_format14() {
        let mut cpu = Arm7Tdmi::new();
        let mut bus = GbaBus::new();
        cpu.regs.cpsr.set_thumb(true);

        cpu.regs.set(REG_SP, 0x0300_7F00);
        cpu.regs.set(0, 0xAAAA);
        cpu.regs.set(1, 0xBBBB);
        cpu.regs.set(REG_LR, 0x0800_0123);

        // PUSH {R0, R1, LR} -> 1011_0_10_1_00000011 = 0xB503
        execute_thumb(&mut cpu, &mut bus, 0xB503);
        assert_eq!(cpu.regs.get(REG_SP), 0x0300_7F00 - 12);
        assert_eq!(bus.read32(0x0300_7F00 - 12), 0xAAAA);
        assert_eq!(bus.read32(0x0300_7F00 - 8), 0xBBBB);
        assert_eq!(bus.read32(0x0300_7F00 - 4), 0x0800_0123);

        // Clear R0, R1
        cpu.regs.set(0, 0);
        cpu.regs.set(1, 0);

        // POP {R0, R1, PC} -> 1011_1_10_1_00000011 = 0xBD03
        execute_thumb(&mut cpu, &mut bus, 0xBD03);
        assert_eq!(cpu.regs.get(0), 0xAAAA);
        assert_eq!(cpu.regs.get(1), 0xBBBB);
        assert_eq!(cpu.regs.get(REG_SP), 0x0300_7F00);
        // PC pop should align and refill pipeline
        assert_eq!(cpu.regs.pc(), 0x0800_0126);
    }
}
