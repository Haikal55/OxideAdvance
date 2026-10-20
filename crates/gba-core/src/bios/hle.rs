use crate::bus::Bus;
use crate::cpu::cpsr::CpuMode;
use crate::cpu::registers::REG_SP;
use crate::cpu::Arm7Tdmi;

/// Menangani panggilan BIOS SWI melalui High-Level Emulation (HLE)
pub fn handle_swi(cpu: &mut Arm7Tdmi, bus: &mut impl Bus, swi_num: u8) -> usize {
    match swi_num {
        // 0x00: SoftReset
        0x00 => {
            cpu.regs.reset();
            cpu.regs.set_pc(0x0800_0000);
            cpu.regs.change_mode(CpuMode::System);
            cpu.regs.cpsr.set_thumb(false);
            cpu.regs.cpsr.set_irq_disabled(false);
            cpu.regs.set(REG_SP, 0x0300_7F00);
            cpu.refill_pipeline(bus);
            3
        }

        // 0x01: RegisterRamReset
        0x01 => {
            let flags = cpu.regs.get(0);
            // bit 0: Clear 256KB EWRAM
            if (flags & 0x01) != 0 {
                for addr in (0x0200_0000..0x0204_0000).step_by(4) {
                    bus.write32(addr, 0);
                }
            }
            // bit 1: Clear 32KB IWRAM (kecuali area BIOS stack di 0x0300_7E00..0x0300_7FFF)
            if (flags & 0x02) != 0 {
                for addr in (0x0300_0000..0x0300_7E00).step_by(4) {
                    bus.write32(addr, 0);
                }
            }
            // bit 2: Clear Palette RAM (1KB)
            if (flags & 0x04) != 0 {
                for addr in (0x0500_0000..0x0500_0400).step_by(4) {
                    bus.write32(addr, 0);
                }
            }
            // bit 3: Clear VRAM (96KB)
            if (flags & 0x08) != 0 {
                for addr in (0x0600_0000..0x0601_8000).step_by(4) {
                    bus.write32(addr, 0);
                }
            }
            // bit 4: Clear OAM (1KB)
            if (flags & 0x10) != 0 {
                for addr in (0x0700_0000..0x0700_0400).step_by(4) {
                    bus.write32(addr, 0);
                }
            }
            3
        }

        // 0x02: Halt
        0x02 => {
            cpu.halted = true;
            1
        }

        // 0x04: IntrWait
        0x04 => {
            bus.write16(0x0400_0208, 1);
            cpu.halted = true;
            1
        }

        // 0x05: VBlankIntrWait
        0x05 => {
            // Pastikan VBlank Interrupt diaktifkan pada IE (0x0400_0200) bit 0
            let ie = bus.read16(0x0400_0200);
            bus.write16(0x0400_0200, ie | 0x0001);
            // Aktifkan Master Interrupt (IME = 1)
            bus.write16(0x0400_0208, 1);
            // Pastikan DISPSTAT VBlank IRQ Enable diset
            let dispstat = bus.read16(0x0400_0004);
            bus.write16(0x0400_0004, dispstat | (1 << 3));
            cpu.halted = true;
            1
        }

        // 0x06: Div (Signed 32-bit: R0 / R1 -> R0=quot, R1=rem, R3=abs(quot))
        0x06 => {
            let num = cpu.regs.get(0) as i32;
            let denom = cpu.regs.get(1) as i32;
            if denom == 0 {
                cpu.regs.set(0, 0);
                cpu.regs.set(1, num as u32);
                cpu.regs.set(3, 0);
            } else {
                let q = num / denom;
                let r = num % denom;
                cpu.regs.set(0, q as u32);
                cpu.regs.set(1, r as u32);
                cpu.regs.set(3, q.abs() as u32);
            }
            3
        }

        // 0x07: DivArm (Signed 32-bit: R1 / R0 -> R0=quot, R1=rem, R3=abs(quot))
        0x07 => {
            let denom = cpu.regs.get(0) as i32;
            let num = cpu.regs.get(1) as i32;
            if denom == 0 {
                cpu.regs.set(0, 0);
                cpu.regs.set(1, num as u32);
                cpu.regs.set(3, 0);
            } else {
                let q = num / denom;
                let r = num % denom;
                cpu.regs.set(0, q as u32);
                cpu.regs.set(1, r as u32);
                cpu.regs.set(3, q.abs() as u32);
            }
            3
        }

        // 0x08: Sqrt
        0x08 => {
            let val = cpu.regs.get(0) as f64;
            let res = val.sqrt() as u32;
            cpu.regs.set(0, res);
            3
        }

        // 0x0B: CpuSet (memcpy / memset)
        0x0B => {
            let src = cpu.regs.get(0);
            let dst = cpu.regs.get(1);
            let cnt = cpu.regs.get(2);

            let count = (cnt & 0x001F_FFFF) as usize;
            let is_fixed = (cnt & (1 << 24)) != 0;
            let is_32bit = (cnt & (1 << 26)) != 0;

            if is_32bit {
                let mut cur_src = src & !3;
                let mut cur_dst = dst & !3;
                for _ in 0..count {
                    let val = bus.read32(cur_src);
                    bus.write32(cur_dst, val);
                    cur_dst = cur_dst.wrapping_add(4);
                    if !is_fixed {
                        cur_src = cur_src.wrapping_add(4);
                    }
                }
            } else {
                let mut cur_src = src & !1;
                let mut cur_dst = dst & !1;
                for _ in 0..count {
                    let val = bus.read16(cur_src);
                    bus.write16(cur_dst, val);
                    cur_dst = cur_dst.wrapping_add(2);
                    if !is_fixed {
                        cur_src = cur_src.wrapping_add(2);
                    }
                }
            }
            3
        }

        // 0x0C: CpuFastSet (fast 32-word block copy/fill)
        0x0C => {
            let src = cpu.regs.get(0);
            let dst = cpu.regs.get(1);
            let cnt = cpu.regs.get(2);

            let raw_count = (cnt & 0x001F_FFFF) as usize;
            // Dibulatkan ke kelipatan 8 word (32 bytes)
            let count = (raw_count + 7) & !7;
            let is_fixed = (cnt & (1 << 24)) != 0;

            let mut cur_src = src & !3;
            let mut cur_dst = dst & !3;
            for _ in 0..count {
                let val = bus.read32(cur_src);
                bus.write32(cur_dst, val);
                cur_dst = cur_dst.wrapping_add(4);
                if !is_fixed {
                    cur_src = cur_src.wrapping_add(4);
                }
            }
            3
        }

        // 0x11: LZ77UnCompWram & 0x12: LZ77UnCompVram
        0x11 | 0x12 => {
            let src = cpu.regs.get(0);
            let dst = cpu.regs.get(1);

            let header = bus.read32(src);
            let dest_len = (header >> 8) as usize;

            let mut written = 0usize;
            let mut cur_src = src.wrapping_add(4);
            let mut cur_dst = dst;

            while written < dest_len {
                let flags = bus.read8(cur_src);
                cur_src = cur_src.wrapping_add(1);

                for bit in (0..8).rev() {
                    if written >= dest_len {
                        break;
                    }
                    if (flags & (1 << bit)) == 0 {
                        let byte = bus.read8(cur_src);
                        cur_src = cur_src.wrapping_add(1);
                        bus.write8(cur_dst, byte);
                        cur_dst = cur_dst.wrapping_add(1);
                        written += 1;
                    } else {
                        let b0 = bus.read8(cur_src) as usize;
                        let b1 = bus.read8(cur_src.wrapping_add(1)) as usize;
                        cur_src = cur_src.wrapping_add(2);

                        let length = (b0 >> 4) + 3;
                        let disp = (((b0 & 0x0F) << 8) | b1) + 1;
                        let mut copy_src = cur_dst.wrapping_sub(disp as u32);

                        for _ in 0..length {
                            if written >= dest_len {
                                break;
                            }
                            let byte = bus.read8(copy_src);
                            copy_src = copy_src.wrapping_add(1);
                            bus.write8(cur_dst, byte);
                            cur_dst = cur_dst.wrapping_add(1);
                            written += 1;
                        }
                    }
                }
            }
            3
        }

        // 0x14: RLUnCompWram & 0x15: RLUnCompVram
        0x14 | 0x15 => {
            let src = cpu.regs.get(0);
            let dst = cpu.regs.get(1);

            let header = bus.read32(src);
            let dest_len = (header >> 8) as usize;

            let mut written = 0usize;
            let mut cur_src = src.wrapping_add(4);
            let mut cur_dst = dst;

            while written < dest_len {
                let flag = bus.read8(cur_src);
                cur_src = cur_src.wrapping_add(1);
                let is_compressed = (flag & 0x80) != 0;
                let count = (flag & 0x7F) as usize;

                if is_compressed {
                    let len = count + 3;
                    let val = bus.read8(cur_src);
                    cur_src = cur_src.wrapping_add(1);
                    for _ in 0..len {
                        if written >= dest_len {
                            break;
                        }
                        bus.write8(cur_dst, val);
                        cur_dst = cur_dst.wrapping_add(1);
                        written += 1;
                    }
                } else {
                    let len = count + 1;
                    for _ in 0..len {
                        if written >= dest_len {
                            break;
                        }
                        let val = bus.read8(cur_src);
                        cur_src = cur_src.wrapping_add(1);
                        bus.write8(cur_dst, val);
                        cur_dst = cur_dst.wrapping_add(1);
                        written += 1;
                    }
                }
            }
            3
        }

        // SWI tak dikenal: kembalikan 1 siklus
        _ => 1,
    }
}
