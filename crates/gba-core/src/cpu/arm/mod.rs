use crate::bus::Bus;
use crate::cpu::alu::{execute_alu, shift_by_imm, shift_by_reg, AluOp, ShiftType};
use crate::cpu::cpsr::CpuMode;
use crate::cpu::registers::{REG_LR, REG_PC};
use crate::cpu::Arm7Tdmi;

/// Menangani dekode dan eksekusi instruksi 32-bit ARMv4T
pub fn execute_arm(cpu: &mut Arm7Tdmi, bus: &mut impl Bus, instruction: u32) -> usize {
    // 4-bit teratas adalah Condition Code (bits 31..=28)
    let condition = instruction >> 28;
    if !cpu.regs.cpsr.satisfies_condition(condition) {
        // Jika kondisi tidak terpenuhi, instruksi diabaikan (1 siklus internal / S-cycle)
        return 1;
    }

    // Cek Software Interrupt (SWI): bits [27:24] == 1111
    if (instruction & 0x0F00_0000) == 0x0F00_0000 {
        return execute_swi(cpu, bus, instruction);
    }

    // Identifikasi kelas instruksi berdasarkan bits [27:25]
    let family = (instruction >> 25) & 0x7;

    match family {
        // 0b000: Data Processing (register), Halfword Transfer, PSR, atau BX
        0b000 => {
            // BX: 0001_0010_1111_1111_1111_0001_Rm
            if (instruction & 0x0FFF_FFF0) == 0x012F_FF10 {
                return execute_bx(cpu, bus, instruction);
            }

            // MRS: Move PSR to Register
            if (instruction & 0x0FBF_0FFF) == 0x010F_0000 {
                return execute_mrs(cpu, instruction);
            }

            // MSR (Register): Move Register to PSR
            if (instruction & 0x0DBF_F000) == 0x0129_F000 || (instruction & 0x0DBF_FFF0) == 0x0128_F000 {
                return execute_msr(cpu, instruction, false);
            }

            // Halfword & Signed Data Transfer: bits [7:4] == 1_S_H_1 (dengan S != 0 || H != 0)
            if (instruction & 0x0000_0090) == 0x0000_0090 && (instruction & 0x0000_0060) != 0 {
                execute_halfword_transfer(cpu, bus, instruction)
            } else if (instruction & 0x0000_0090) == 0x0000_0090 {
                // Multiply / Multiply Long (MUL, MLA, UMULL, etc.)
                execute_multiply(cpu, instruction)
            } else {
                execute_data_processing(cpu, bus, instruction, false)
            }
        }

        // 0b001: Data Processing (Immediate) atau MSR (Immediate)
        0b001 => {
            let is_test_op = {
                let op = (instruction >> 21) & 0xF;
                (8..=11).contains(&op)
            };
            let set_flags = (instruction & (1 << 20)) != 0;

            if is_test_op && !set_flags {
                execute_msr(cpu, instruction, true)
            } else {
                execute_data_processing(cpu, bus, instruction, true)
            }
        }

        // 0b010 & 0b011: Single Data Transfer (LDR, STR, LDRB, STRB)
        // 0b010: Immediate offset, 0b011: Register shifted offset
        0b010 | 0b011 => {
            execute_single_data_transfer(cpu, bus, instruction)
        }

        // 0b100: Block Data Transfer (LDM, STM)
        0b100 => {
            execute_block_data_transfer(cpu, bus, instruction)
        }

        // 0b101: Branch / Branch with Link (B, BL)
        0b101 => {
            let is_link = (instruction & (1 << 24)) != 0;
            let mut offset = (instruction & 0x00FF_FFFF) as i32;
            if (offset & 0x0080_0000) != 0 {
                offset |= !0x00FF_FFFF;
            }
            offset <<= 2;

            if is_link {
                let next_instr = cpu.regs.pc().wrapping_sub(4);
                cpu.regs.set(REG_LR, next_instr);
            }

            let target = (cpu.regs.pc() as i32).wrapping_add(offset) as u32;
            cpu.regs.set_pc(target);
            cpu.refill_pipeline(bus);
            3
        }

        _ => 1,
    }
}

/// Eksekusi Single Data Transfer (LDR, STR, LDRB, STRB)
fn execute_single_data_transfer(cpu: &mut Arm7Tdmi, bus: &mut impl Bus, instruction: u32) -> usize {
    // Pada Single Data Transfer: bit 25 = 0 adalah immediate, bit 25 = 1 adalah register-shifted!
    let is_reg_offset = (instruction & (1 << 25)) != 0;
    let pre_index = (instruction & (1 << 24)) != 0;
    let add_offset = (instruction & (1 << 23)) != 0;
    let byte_transfer = (instruction & (1 << 22)) != 0;
    let write_back = (instruction & (1 << 21)) != 0;
    let is_load = (instruction & (1 << 20)) != 0;
    let rn = ((instruction >> 16) & 0xF) as usize;
    let rd = ((instruction >> 12) & 0xF) as usize;

    let offset = if is_reg_offset {
        let rm = (instruction & 0xF) as usize;
        let rm_val = cpu.regs.get(rm);
        let shift_amt = (instruction >> 7) & 0x1F;
        let shift_type = ShiftType::from_bits((instruction >> 5) & 0x3);
        let (val, _) = shift_by_imm(rm_val, shift_type, shift_amt, cpu.regs.cpsr.c());
        val
    } else {
        instruction & 0xFFF
    };

    let base = cpu.regs.get(rn);
    let addr_offset = if add_offset {
        base.wrapping_add(offset)
    } else {
        base.wrapping_sub(offset)
    };

    let transfer_addr = if pre_index { addr_offset } else { base };
    let do_write_back = !pre_index || write_back;

    if is_load {
        let val = if byte_transfer {
            bus.read8(transfer_addr) as u32
        } else {
            bus.read32_unaligned(transfer_addr)
        };

        if do_write_back && rn != rd {
            cpu.regs.set(rn, addr_offset);
        }

        cpu.regs.set(rd, val);

        if rd == REG_PC {
            cpu.refill_pipeline(bus);
            return 5;
        }
        3
    } else {
        let mut val = cpu.regs.get(rd);
        if rd == REG_PC {
            // Pada ARM7TDMI: STR PC menyimpan PC + 12 (karena pipeline saat ini bernilai PC + 8)
            val = val.wrapping_add(4);
        }

        if byte_transfer {
            bus.write8(transfer_addr, val as u8);
        } else {
            bus.write32(transfer_addr & !3, val);
        }

        if do_write_back {
            cpu.regs.set(rn, addr_offset);
        }
        2
    }
}

/// Eksekusi Halfword & Signed Data Transfer (LDRH, STRH, LDRSB, LDRSH)
fn execute_halfword_transfer(cpu: &mut Arm7Tdmi, bus: &mut impl Bus, instruction: u32) -> usize {
    let pre_index = (instruction & (1 << 24)) != 0;
    let add_offset = (instruction & (1 << 23)) != 0;
    let is_imm = (instruction & (1 << 22)) != 0; // Bit 22 adalah flag immediate untuk format ini
    let write_back = (instruction & (1 << 21)) != 0;
    let is_load = (instruction & (1 << 20)) != 0;
    let rn = ((instruction >> 16) & 0xF) as usize;
    let rd = ((instruction >> 12) & 0xF) as usize;
    let s = (instruction & (1 << 6)) != 0;
    let h = (instruction & (1 << 5)) != 0;

    let offset = if is_imm {
        let high = (instruction >> 8) & 0xF;
        let low = instruction & 0xF;
        (high << 4) | low
    } else {
        let rm = (instruction & 0xF) as usize;
        cpu.regs.get(rm)
    };

    let base = cpu.regs.get(rn);
    let addr_offset = if add_offset {
        base.wrapping_add(offset)
    } else {
        base.wrapping_sub(offset)
    };

    let transfer_addr = if pre_index { addr_offset } else { base };
    let do_write_back = !pre_index || write_back;

    if is_load {
        let val = match (s, h) {
            (false, true) => {
                // LDRH (Unsigned Halfword)
                bus.read16_unaligned(transfer_addr) as u32
            }
            (true, false) => {
                // LDRSB (Signed Byte)
                (bus.read8(transfer_addr) as i8 as i32) as u32
            }
            (true, true) => {
                // LDRSH (Signed Halfword)
                if (transfer_addr & 1) == 0 {
                    (bus.read16(transfer_addr) as i16 as i32) as u32
                } else {
                    // Karakteristik ARMv4T: unaligned LDRSH membaca byte dan sign-extend
                    (bus.read8(transfer_addr) as i8 as i32) as u32
                }
            }
            _ => 0,
        };

        if do_write_back && rn != rd {
            cpu.regs.set(rn, addr_offset);
        }

        cpu.regs.set(rd, val);

        if rd == REG_PC {
            cpu.refill_pipeline(bus);
            return 5;
        }
        3
    } else {
        // STRH
        let mut val = cpu.regs.get(rd);
        if rd == REG_PC {
            val = val.wrapping_add(4);
        }
        bus.write16(transfer_addr & !1, val as u16);

        if do_write_back {
            cpu.regs.set(rn, addr_offset);
        }
        2
    }
}

/// Eksekusi Block Data Transfer (LDM, STM)
fn execute_block_data_transfer(cpu: &mut Arm7Tdmi, bus: &mut impl Bus, instruction: u32) -> usize {
    let pre_index = (instruction & (1 << 24)) != 0;
    let add_offset = (instruction & (1 << 23)) != 0;
    let s_bit = (instruction & (1 << 22)) != 0;
    let write_back = (instruction & (1 << 21)) != 0;
    let is_load = (instruction & (1 << 20)) != 0;
    let rn = ((instruction >> 16) & 0xF) as usize;
    let rlist = instruction & 0xFFFF;

    let base = cpu.regs.get(rn);
    let count = rlist.count_ones();

    // Hardware quirk: Empty register list
    if count == 0 {
        let addr = if add_offset {
            if pre_index { base.wrapping_add(4) } else { base }
        } else if pre_index {
            base.wrapping_sub(0x40)
        } else {
            base.wrapping_sub(0x3C)
        };

        if is_load {
            let val = bus.read32(addr & !3);
            cpu.regs.set(REG_PC, val);
            cpu.refill_pipeline(bus);
        } else {
            bus.write32(addr & !3, cpu.regs.pc().wrapping_add(4));
        }

        if write_back {
            let new_rn = if add_offset { base.wrapping_add(0x40) } else { base.wrapping_sub(0x40) };
            cpu.regs.set(rn, new_rn);
        }
        return 3;
    }

    let total_bytes = count * 4;
    let start_addr = if add_offset {
        if pre_index { base.wrapping_add(4) } else { base }
    } else if pre_index {
        base.wrapping_sub(total_bytes)
    } else {
        base.wrapping_sub(total_bytes).wrapping_add(4)
    };

    let new_rn = if add_offset { base.wrapping_add(total_bytes) } else { base.wrapping_sub(total_bytes) };

    let has_pc = (rlist & (1 << REG_PC)) != 0;
    let force_user = s_bit && (!is_load || !has_pc);

    let mut current_addr = start_addr;
    let mut loaded_rn = false;

    for i in 0..16 {
        if (rlist & (1 << i)) != 0 {
            if is_load {
                let val = bus.read32(current_addr & !3);
                if force_user {
                    cpu.regs.set_user(i, val);
                } else {
                    cpu.regs.set(i, val);
                }
                if i == rn {
                    loaded_rn = true;
                }
            } else {
                let mut val = if force_user {
                    cpu.regs.get_user(i)
                } else {
                    cpu.regs.get(i)
                };
                if i == REG_PC {
                    val = val.wrapping_add(4);
                }
                bus.write32(current_addr & !3, val);
            }
            current_addr = current_addr.wrapping_add(4);
        }
    }

    if write_back && !loaded_rn {
        cpu.regs.set(rn, new_rn);
    }

    if is_load && has_pc {
        if s_bit {
            let spsr = cpu.regs.get_spsr();
            if let Some(new_mode) = CpuMode::from_bits(spsr) {
                cpu.regs.change_mode(new_mode);
            }
            cpu.regs.cpsr.value = spsr;
        }
        cpu.refill_pipeline(bus);
        return (count as usize) + 3;
    }

    (count as usize) + 1
}

/// Eksekusi instruksi Multiply (MUL, MLA)
fn execute_multiply(cpu: &mut Arm7Tdmi, instruction: u32) -> usize {
    let set_flags = (instruction & (1 << 20)) != 0;
    let is_accumulate = (instruction & (1 << 21)) != 0;
    let rd = ((instruction >> 16) & 0xF) as usize;
    let rn = ((instruction >> 12) & 0xF) as usize;
    let rs = ((instruction >> 8) & 0xF) as usize;
    let rm = (instruction & 0xF) as usize;

    let rm_val = cpu.regs.get(rm);
    let rs_val = cpu.regs.get(rs);
    let rn_val = if is_accumulate { cpu.regs.get(rn) } else { 0 };

    let res = rm_val.wrapping_mul(rs_val).wrapping_add(rn_val);
    cpu.regs.set(rd, res);

    if set_flags {
        cpu.regs.cpsr.set_n((res & 0x8000_0000) != 0);
        cpu.regs.cpsr.set_z(res == 0);
    }

    3
}

/// Eksekusi SWI: Software Interrupt
fn execute_swi(cpu: &mut Arm7Tdmi, bus: &mut impl Bus, instruction: u32) -> usize {
    if bus.is_bios_hle() {
        let mut swi_num = ((instruction >> 16) & 0xFF) as u8;
        if swi_num == 0 {
            swi_num = (instruction & 0xFF) as u8;
        }
        return crate::bios::hle::handle_swi(cpu, bus, swi_num);
    }

    let cpsr = cpu.regs.cpsr.value;
    let return_pc = cpu.regs.pc().wrapping_sub(4);

    cpu.regs.change_mode(CpuMode::Supervisor);
    cpu.regs.set_spsr(cpsr);
    cpu.regs.set(REG_LR, return_pc);

    cpu.regs.cpsr.set_thumb(false);
    cpu.regs.cpsr.set_irq_disabled(true);

    cpu.regs.set_pc(0x0000_0008);
    cpu.refill_pipeline(bus);
    3
}

/// Eksekusi instruksi Data Processing (16 operasi ALU ARM)
fn execute_data_processing(
    cpu: &mut Arm7Tdmi,
    bus: &mut impl Bus,
    instruction: u32,
    is_imm: bool,
) -> usize {
    let opcode = AluOp::from_bits((instruction >> 21) & 0xF);
    let set_flags = (instruction & (1 << 20)) != 0;
    let rn = ((instruction >> 16) & 0xF) as usize;
    let rd = ((instruction >> 12) & 0xF) as usize;

    let (op2, shifter_carry, extra_cycles) = if is_imm {
        let imm8 = instruction & 0xFF;
        let rotate = ((instruction >> 8) & 0xF) * 2;
        let val = imm8.rotate_right(rotate);
        let carry = if rotate == 0 {
            cpu.regs.cpsr.c()
        } else {
            (val & 0x8000_0000) != 0
        };
        (val, carry, 0)
    } else {
        let rm = (instruction & 0xF) as usize;
        let mut rm_val = cpu.regs.get(rm);
        let shift_type = ShiftType::from_bits((instruction >> 5) & 0x3);
        let is_reg_shift = (instruction & (1 << 4)) != 0;

        if is_reg_shift {
            let rs = ((instruction >> 8) & 0xF) as usize;
            let rs_val = cpu.regs.get(rs);
            if rm == REG_PC {
                rm_val = rm_val.wrapping_add(4);
            }
            let (val, carry) = shift_by_reg(rm_val, shift_type, rs_val, cpu.regs.cpsr.c());
            (val, carry, 1)
        } else {
            let shift_imm = (instruction >> 7) & 0x1F;
            let (val, carry) = shift_by_imm(rm_val, shift_type, shift_imm, cpu.regs.cpsr.c());
            (val, carry, 0)
        }
    };

    let op1 = cpu.regs.get(rn);
    let alu_out = execute_alu(opcode, op1, op2, cpu.regs.cpsr.c(), shifter_carry);

    if !opcode.is_test() {
        cpu.regs.set(rd, alu_out.result);
    }

    if set_flags {
        if rd == REG_PC {
            let spsr = cpu.regs.get_spsr();
            if let Some(new_mode) = CpuMode::from_bits(spsr) {
                cpu.regs.change_mode(new_mode);
            }
            cpu.regs.cpsr.value = spsr;
        } else {
            cpu.regs.cpsr.set_n(alu_out.n);
            cpu.regs.cpsr.set_z(alu_out.z);
            cpu.regs.cpsr.set_c(alu_out.c);
            if let Some(v) = alu_out.v {
                cpu.regs.cpsr.set_v(v);
            }
        }
    }

    if rd == REG_PC {
        cpu.refill_pipeline(bus);
        return 3;
    }

    1 + extra_cycles
}

/// Eksekusi instruksi BX: Branch and Exchange (mengganti mode ARM <-> THUMB)
fn execute_bx(cpu: &mut Arm7Tdmi, bus: &mut impl Bus, instruction: u32) -> usize {
    let rm = (instruction & 0xF) as usize;
    let target = cpu.regs.get(rm);

    let is_thumb = (target & 1) != 0;
    cpu.regs.cpsr.set_thumb(is_thumb);

    let target_pc = if is_thumb {
        target & !1
    } else {
        target & !3
    };

    cpu.regs.set_pc(target_pc);
    cpu.refill_pipeline(bus);
    3
}

/// Eksekusi instruksi MRS: Move PSR to Register
fn execute_mrs(cpu: &mut Arm7Tdmi, instruction: u32) -> usize {
    let use_spsr = (instruction & (1 << 22)) != 0;
    let rd = ((instruction >> 12) & 0xF) as usize;

    let val = if use_spsr {
        cpu.regs.get_spsr()
    } else {
        cpu.regs.cpsr.value
    };

    cpu.regs.set(rd, val);
    1
}

/// Eksekusi instruksi MSR: Move to Status Register from ARM register or immediate
fn execute_msr(cpu: &mut Arm7Tdmi, instruction: u32, is_imm: bool) -> usize {
    let use_spsr = (instruction & (1 << 22)) != 0;

    let operand = if is_imm {
        let imm8 = instruction & 0xFF;
        let rotate = ((instruction >> 8) & 0xF) * 2;
        imm8.rotate_right(rotate)
    } else {
        let rm = (instruction & 0xF) as usize;
        cpu.regs.get(rm)
    };

    let mut mask = 0u32;
    if (instruction & (1 << 16)) != 0 { mask |= 0x0000_00FF; }
    if (instruction & (1 << 17)) != 0 { mask |= 0x0000_FF00; }
    if (instruction & (1 << 18)) != 0 { mask |= 0x00FF_0000; }
    if (instruction & (1 << 19)) != 0 { mask |= 0xFF00_0000; }

    if use_spsr {
        let current = cpu.regs.get_spsr();
        let new_spsr = (current & !mask) | (operand & mask);
        cpu.regs.set_spsr(new_spsr);
    } else {
        if cpu.regs.cpsr.mode() == CpuMode::User {
            mask &= 0xFF00_0000;
        }

        let current = cpu.regs.cpsr.value;
        let new_cpsr = (current & !mask) | (operand & mask);

        if (mask & 0x1F) != 0 {
            if let Some(new_mode) = CpuMode::from_bits(new_cpsr) {
                cpu.regs.change_mode(new_mode);
            }
        }
        cpu.regs.cpsr.value = new_cpsr;
    }

    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::GbaBus;

    #[test]
    fn test_arm_mov_and_add_immediate() {
        let mut cpu = Arm7Tdmi::new();
        let mut bus = GbaBus::new();

        let instr_mov = 0xE3A0_002A; // MOV R0, #42
        execute_arm(&mut cpu, &mut bus, instr_mov);
        assert_eq!(cpu.regs.get(0), 42);

        let instr_add = 0xE290_100A; // ADD R1, R0, #10 (S=1)
        execute_arm(&mut cpu, &mut bus, instr_add);
        assert_eq!(cpu.regs.get(1), 52);
        assert!(!cpu.regs.cpsr.z());
        assert!(!cpu.regs.cpsr.c());
    }

    #[test]
    fn test_arm_cmp_sets_zero_flag() {
        let mut cpu = Arm7Tdmi::new();
        let mut bus = GbaBus::new();
        cpu.regs.set(0, 100);

        let instr_cmp = 0xE350_0064; // CMP R0, #100
        execute_arm(&mut cpu, &mut bus, instr_cmp);
        assert!(cpu.regs.cpsr.z());
        assert!(cpu.regs.cpsr.c());
    }

    #[test]
    fn test_arm_bx_switches_to_thumb() {
        let mut cpu = Arm7Tdmi::new();
        let mut bus = GbaBus::new();
        cpu.regs.set(1, 0x0800_0101); // Bit 0 = 1 (Thumb)

        let instr_bx = 0xE12F_FF11;
        execute_arm(&mut cpu, &mut bus, instr_bx);

        assert!(cpu.regs.cpsr.is_thumb());
        assert_eq!(cpu.regs.pc(), 0x0800_0104);
    }

    #[test]
    fn test_arm_str_and_ldr() {
        let mut cpu = Arm7Tdmi::new();
        let mut bus = GbaBus::new();

        cpu.regs.set(0, 0x0300_0000); // Base IWRAM
        cpu.regs.set(1, 0xDEAD_BEEF); // Data

        // STR R1, [R0, #4] (Pre-index, Up, Word) -> 0xE5801004
        let instr_str = 0xE580_1004;
        execute_arm(&mut cpu, &mut bus, instr_str);
        assert_eq!(bus.read32(0x0300_0004), 0xDEAD_BEEF);

        // LDR R2, [R0, #4] -> 0xE5902004
        let instr_ldr = 0xE590_2004;
        execute_arm(&mut cpu, &mut bus, instr_ldr);
        assert_eq!(cpu.regs.get(2), 0xDEAD_BEEF);
    }

    #[test]
    fn test_arm_block_data_transfer_stm_ldm() {
        let mut cpu = Arm7Tdmi::new();
        let mut bus = GbaBus::new();

        cpu.regs.set(13, 0x0300_1000); // SP
        cpu.regs.set(0, 0x1111);
        cpu.regs.set(1, 0x2222);
        cpu.regs.set(2, 0x3333);

        // STMIA SP!, {R0, R1, R2} (P=0, U=1, W=1, L=0) -> 0xE8A0_0007 (wait, Rn=13 is 0xD: 0xE8AD0007)
        let instr_stm = 0xE8AD_0007;
        execute_arm(&mut cpu, &mut bus, instr_stm);

        assert_eq!(bus.read32(0x0300_1000), 0x1111);
        assert_eq!(bus.read32(0x0300_1004), 0x2222);
        assert_eq!(bus.read32(0x0300_1008), 0x3333);
        assert_eq!(cpu.regs.get(13), 0x0300_100C); // Updated SP

        // Clear R0, R1, R2
        cpu.regs.set(0, 0);
        cpu.regs.set(1, 0);
        cpu.regs.set(2, 0);

        // LDMDB SP!, {R0, R1, R2} (P=1, U=0, W=1, L=1) -> 0xE93D0007
        let instr_ldm = 0xE93D_0007;
        execute_arm(&mut cpu, &mut bus, instr_ldm);

        assert_eq!(cpu.regs.get(0), 0x1111);
        assert_eq!(cpu.regs.get(1), 0x2222);
        assert_eq!(cpu.regs.get(2), 0x3333);
        assert_eq!(cpu.regs.get(13), 0x0300_1000); // Restored SP
    }
}
