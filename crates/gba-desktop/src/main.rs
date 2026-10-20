use gba_core::bus::Bus;
use gba_core::ppu::{CYCLES_PER_FRAME, SCREEN_HEIGHT, SCREEN_WIDTH};
use gba_core::{Gba, GbaKey};
use minifb::{Key, Scale, Window, WindowOptions};
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

/// Audio Player yang mengalirkan PCM stream 44.1 kHz stereo ke ALSA aplay secara non-blocking
pub struct AudioPlayer {
    sender: Option<SyncSender<Vec<f32>>>,
}

impl AudioPlayer {
    pub fn new() -> Self {
        // Jalankan aplay dengan parameter stereo 44.1 kHz Float32 Little-Endian
        let child = Command::new("aplay")
            .args(["-t", "raw", "-f", "FLOAT_LE", "-c", "2", "-r", "44100", "-q", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();

        match child {
            Ok(mut process) => {
                if let Some(mut stdin) = process.stdin.take() {
                    let (tx, rx) = sync_channel::<Vec<f32>>(16);

                    thread::spawn(move || {
                        let mut byte_buf = Vec::new();
                        while let Ok(samples) = rx.recv() {
                            byte_buf.clear();
                            for s in samples {
                                byte_buf.extend_from_slice(&s.to_le_bytes());
                            }
                            if stdin.write_all(&byte_buf).is_err() {
                                break;
                            }
                        }
                    });

                    println!("[Audio] Output terhubung: ALSA PCM Stream (44.1 kHz, Stereo).");
                    Self { sender: Some(tx) }
                } else {
                    println!("[Audio] Gagal membuka stdin audio player.");
                    Self { sender: None }
                }
            }
            Err(_) => {
                println!("[Audio] aplay tidak tersedia, menjalankan emulator dalam mode silent.");
                Self { sender: None }
            }
        }
    }

    pub fn play(&self, samples: &[(f32, f32)]) {
        if let Some(ref tx) = self.sender {
            if !samples.is_empty() {
                let mut flat = Vec::with_capacity(samples.len() * 2);
                for &(l, r) in samples {
                    flat.push(l);
                    flat.push(r);
                }
                let _ = tx.try_send(flat);
            }
        }
    }
}

/// Mainkan nada retro PSG Channel 1 pada demo mode
fn play_demo_tone(gba: &mut Gba, freq_divider: u16) {
    // Sound Master Control: enable all sound (0x0400_0084 = 0x0080)
    gba.bus.write16(0x0400_0084, 0x0080);
    // Sound Output: full volume L & R untuk Channel 1 (0x0400_0080 = 0x0077)
    gba.bus.write16(0x0400_0080, 0x0077);
    gba.bus.write16(0x0400_0082, 0x000F);
    // Ch1 Control: 50% duty, initial volume 15, envelope decay
    gba.bus.write16(0x0400_0062, 0xF380);
    // Ch1 Frequency & Trigger (bit 15 = restart)
    gba.bus.write16(0x0400_0064, 0x8000 | (freq_divider & 0x07FF));
}

fn setup_mode0_demo(gba: &mut Gba) {
    // Nonaktifkan seluruh 128 sprite secara default (bit 9 attr0 = 0x0200)
    for i in 0..128 {
        gba.bus.write16(0x0700_0000 + i * 8, 0x0200);
    }

    // DISPCNT: Mode 0 + Enable BG0 (bit 8 = 0x0100) + Enable OBJ (bit 12 = 0x1000) -> 0x1100
    gba.bus.write16(0x0400_0000, 0x1100);

    // BG0CNT: Priority 1, CharBase 0 (0x0000), 4bpp (bit 7 = 0), ScreenBase 2 (0x1000), 32x32 (00) -> 0x0201
    gba.bus.write16(0x0400_0008, 0x0201);

    // Setup Palette RAM Background (0x0500_0000):
    // Index 0: Backdrop (Navy Blue)
    gba.bus.write16(0x0500_0000, 0x3800);
    // Palette 0, Index 1: Putih (0x7FFF)
    gba.bus.write16(0x0500_0002, 0x7FFF);
    // Palette 0, Index 2: Hijau Rumput (0x0340)
    gba.bus.write16(0x0500_0004, 0x0340);
    // Palette 0, Index 3: Cokelat Bata (0x015A)
    gba.bus.write16(0x0500_0006, 0x015A);
    // Palette 0, Index 4: Emas / Kuning (0x03FF)
    gba.bus.write16(0x0500_0008, 0x03FF);

    // Setup Palette RAM Sprites (0x0500_0200):
    // Palette 0, Index 1: Kuning Terang (0x03FF)
    gba.bus.write16(0x0500_0202, 0x03FF);
    // Palette 0, Index 2: Merah Ruby (0x001F)
    gba.bus.write16(0x0500_0204, 0x001F);
    // Palette 0, Index 3: Putih Berlian (0x7FFF)
    gba.bus.write16(0x0500_0206, 0x7FFF);
    // Palette 0, Index 4: Hitam Outline (0x0000)
    gba.bus.write16(0x0500_0208, 0x0000);

    // Buat 8x8 Sprite Tile 0 di CharBase 4 (0x0601_0000) berupa karakter kursor bintang/berlian:
    for row in 0..8 {
        let pattern = match row {
            0 | 7 => 0x00,
            1 | 6 => 0x22,
            2 | 5 => 0x13,
            3 | 4 => 0x31,
            _ => 0x00,
        };
        for col in 0..4 {
            let addr = 0x0601_0000 + (row * 4 + col);
            gba.bus.write8(addr, pattern);
        }
    }

    // Aktifkan OBJ 0: Y = 76, X = 116 (tengah layar), 8x8, 4bpp, Priority 0 (di depan BG1), Palette 0
    gba.bus.write16(0x0700_0000, 76);  // Attr0: Y=76, Normal, Enabled
    gba.bus.write16(0x0700_0002, 116); // Attr1: X=116, Size 8x8
    gba.bus.write16(0x0700_0004, 0);   // Attr2: Tile 0, Priority 0, Palette 0

    // Buat 8x8 Tile 0: Checkerboard pattern (32 bytes pada 0x0600_0000)
    for row in 0..8 {
        let pattern = if row % 2 == 0 { 0x12 } else { 0x21 };
        for col in 0..4 {
            let addr = 0x0600_0000 + (row * 4 + col);
            gba.bus.write8(addr, pattern);
        }
    }

    // Buat 8x8 Tile 1: Brick Wall pattern (32 bytes pada 0x0600_0020)
    for row in 0..8 {
        let pattern = if row == 0 || row == 4 { 0x11 } else { 0x33 };
        for col in 0..4 {
            let addr = 0x0600_0020 + (row * 4 + col);
            gba.bus.write8(addr, pattern);
        }
    }

    // Buat 8x8 Tile 2: Star / Coin pattern (32 bytes pada 0x0600_0040)
    for row in 0..8 {
        let pattern = if (2..=5).contains(&row) { 0x44 } else { 0x00 };
        for col in 0..4 {
            let addr = 0x0600_0040 + (row * 4 + col);
            gba.bus.write8(addr, pattern);
        }
    }

    // Isi ScreenBase 2 (32x32 = 1024 entri tile pada 0x0600_1000)
    for ty in 0..32u32 {
        for tx in 0..32u32 {
            let tile_num = if (tx + ty) % 5 == 0 {
                2 // Koin
            } else if (tx % 4 == 0) || (ty % 4 == 0) {
                1 // Bata
            } else {
                0 // Kotak-kotak
            };

            let entry = tile_num as u16;
            let addr = 0x0600_1000 + ((ty * 32 + tx) * 2);
            gba.bus.write16(addr, entry);
        }
    }

    // Tulis instruksi loop ARM B . (0xEAFF_FFFE) di BIOS 0x0000_0000 agar CPU diam di tempat
    gba.bus.write32(0x0000_0000, 0xEAFF_FFFE);
    gba.bus.write32(0x0000_0004, 0xEAFF_FFFE);
    gba.reset();

    // Bunyikan nada intro pembuka (C5)
    play_demo_tone(gba, 0x0705);
}

fn main() {
    println!("========================================================");
    println!("  OxideAdvance - Pure Rust Game Boy Advance (GBA) Emulator");
    println!("  Core: ARM7TDMI 32-bit (ARMv4T) + PPU + MMU + APU Audio");
    println!("========================================================");

    let mut gba = Gba::new();
    let audio_player = AudioPlayer::new();

    let args: Vec<String> = env::args().collect();
    let mut rom_title_str = String::from("Built-in Demo");
    let is_rom_loaded = if args.len() > 1 {
        let rom_path = &args[1];
        match fs::read(rom_path) {
            Ok(rom_data) => {
                println!("Memuat ROM: {} ({} bytes)...", rom_path, rom_data.len());
                gba.bus.load_rom(&rom_data);
                gba.reset();

                // Ambil judul game dari GBA Cartridge Header (offset 0xA0..0xAC)
                if rom_data.len() >= 0xAC {
                    let title_bytes = &rom_data[0xA0..0xAC];
                    let end_pos = title_bytes.iter().position(|&b| b == 0).unwrap_or(12);
                    if let Ok(title) = std::str::from_utf8(&title_bytes[..end_pos]) {
                        let clean = title.trim();
                        if !clean.is_empty() {
                            rom_title_str = clean.to_string();
                        }
                    }
                }
                if rom_title_str == "Built-in Demo" {
                    if let Some(file_name) = Path::new(rom_path).file_name().and_then(|n| n.to_str()) {
                        rom_title_str = file_name.to_string();
                    }
                }
                println!("ROM Berhasil Dimuat: [{}]", rom_title_str);
                true
            }
            Err(e) => {
                eprintln!("Gagal membaca file ROM '{}': {}", rom_path, e);
                false
            }
        }
    } else {
        println!("Tidak ada ROM eksternal yang dimuat.");
        println!("Menjalankan Mode Interaktif Built-in (Demo):");
        println!("  Tekan [TAB] untuk berganti antara Mode 0 (Tile Engine) & Mode 3 (Direct Color Bitmap)!");
        println!("  Tekan [Z] atau [X] untuk memainkan nada audio retro!");
        false
    };

    let mut current_demo_mode = 0; // 0 = Mode 0 (Tilemap), 3 = Mode 3 (Bitmap)
    if !is_rom_loaded {
        setup_mode0_demo(&mut gba);
    }

    let window_title = format!("OxideAdvance - Pure Rust GBA Emulator | {}", rom_title_str);
    let mut window = match Window::new(
        &window_title,
        SCREEN_WIDTH,
        SCREEN_HEIGHT,
        WindowOptions {
            scale: Scale::X4,
            resize: false,
            ..WindowOptions::default()
        },
    ) {
        Ok(win) => win,
        Err(err) => {
            eprintln!("Gagal menginisialisasi GUI Window: {}", err);
            return;
        }
    };

    window.set_target_fps(60);

    let mut last_fps_time = Instant::now();
    let mut frame_count = 0;
    let mut tab_was_down = false;
    let mut a_was_down = false;
    let mut b_was_down = false;
    let mut start_was_down = false;

    // State demo
    let mut scroll_x: u16 = 0;
    let mut scroll_y: u16 = 0;
    let mut sprite_x: i32 = 116;
    let mut sprite_y: i32 = 76;
    let mut cursor_x: i32 = 120;
    let mut cursor_y: i32 = 80;
    let mut color_hue: u16 = 0;

    println!("\nKontrol Keyboard:");
    println!("  D-Pad       : Panah Atas, Bawah, Kiri, Kanan");
    println!("  Tombol A    : Z (Mainkan Nada C5 pada Demo)");
    println!("  Tombol B    : X (Mainkan Nada E5 pada Demo)");
    println!("  Start       : Enter (Mainkan Nada A5 pada Demo)");
    println!("  Select      : Backspace / Spasi");
    println!("  Shoulder L/R: A / S");
    println!("  Ganti Mode  : TAB (Mode 0 Tilemap <-> Mode 3 Bitmap)");
    println!("  Keluar      : Escape");

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let btn_a = window.is_key_down(Key::Z);
        let btn_b = window.is_key_down(Key::X);
        let btn_start = window.is_key_down(Key::Enter);
        let btn_select = window.is_key_down(Key::Backspace) || window.is_key_down(Key::Space);
        let dpad_up = window.is_key_down(Key::Up);
        let dpad_down = window.is_key_down(Key::Down);
        let dpad_left = window.is_key_down(Key::Left);
        let dpad_right = window.is_key_down(Key::Right);
        let btn_l = window.is_key_down(Key::A);
        let btn_r = window.is_key_down(Key::S);
        let tab_is_down = window.is_key_down(Key::Tab);

        // Demo Mode Audio Tones & Controls
        if !is_rom_loaded {
            if btn_a && !a_was_down {
                play_demo_tone(&mut gba, 0x0705); // Nada C5 (523 Hz)
            }
            if btn_b && !b_was_down {
                play_demo_tone(&mut gba, 0x0739); // Nada E5 (659 Hz)
            }
            if btn_start && !start_was_down {
                play_demo_tone(&mut gba, 0x076B); // Nada A5 (880 Hz)
            }

            // Toggle antara Mode 0 dan Mode 3 saat TAB ditekan
            if tab_is_down && !tab_was_down {
                if current_demo_mode == 0 {
                    current_demo_mode = 3;
                    // Aktifkan Mode 3 (0x0403: Mode 3 + BG2 enable)
                    gba.bus.write16(0x0400_0000, 0x0403);
                    play_demo_tone(&mut gba, 0x06D6); // Mode switch tone
                } else {
                    current_demo_mode = 0;
                    setup_mode0_demo(&mut gba);
                }
            }
        }
        tab_was_down = tab_is_down;
        a_was_down = btn_a;
        b_was_down = btn_b;
        start_was_down = btn_start;

        // Kirim status tombol ke register KEYINPUT hardware GBA
        gba.bus.keypad.set_key_pressed(GbaKey::A, btn_a);
        gba.bus.keypad.set_key_pressed(GbaKey::B, btn_b);
        gba.bus.keypad.set_key_pressed(GbaKey::Start, btn_start);
        gba.bus.keypad.set_key_pressed(GbaKey::Select, btn_select);
        gba.bus.keypad.set_key_pressed(GbaKey::Up, dpad_up);
        gba.bus.keypad.set_key_pressed(GbaKey::Down, dpad_down);
        gba.bus.keypad.set_key_pressed(GbaKey::Left, dpad_left);
        gba.bus.keypad.set_key_pressed(GbaKey::Right, dpad_right);
        gba.bus.keypad.set_key_pressed(GbaKey::L, btn_l);
        gba.bus.keypad.set_key_pressed(GbaKey::R, btn_r);

        if !is_rom_loaded {
            if current_demo_mode == 0 {
                // Mode 0: Hardware Scrolling BG0 + Posisi Hardware Sprite OBJ 0
                if dpad_left {
                    scroll_x = scroll_x.wrapping_sub(1);
                    if sprite_x > 2 { sprite_x -= 2; }
                }
                if dpad_right {
                    scroll_x = scroll_x.wrapping_add(1);
                    if sprite_x < (SCREEN_WIDTH as i32 - 10) { sprite_x += 2; }
                }
                if dpad_up {
                    scroll_y = scroll_y.wrapping_sub(1);
                    if sprite_y > 2 { sprite_y -= 2; }
                }
                if dpad_down {
                    scroll_y = scroll_y.wrapping_add(1);
                    if sprite_y < (SCREEN_HEIGHT as i32 - 10) { sprite_y += 2; }
                }

                // Tulis ke register BG0HOFS (0x0400_0010) dan BG0VOFS (0x0400_0012)
                gba.bus.write16(0x0400_0010, scroll_x);
                gba.bus.write16(0x0400_0012, scroll_y);

                // Perbarui posisi Hardware Sprite OBJ 0 di OAM (0x0700_0000)
                gba.bus.write16(0x0700_0000, sprite_y as u16);
                gba.bus.write16(0x0700_0002, sprite_x as u16);
            } else {
                // Mode 3: Direct Bitmap Color Plasma & Cursor
                if dpad_left && cursor_x > 8 { cursor_x -= 2; }
                if dpad_right && cursor_x < (SCREEN_WIDTH as i32 - 12) { cursor_x += 2; }
                if dpad_up && cursor_y > 8 { cursor_y -= 2; }
                if dpad_down && cursor_y < (SCREEN_HEIGHT as i32 - 12) { cursor_y += 2; }
                if btn_l { color_hue = color_hue.wrapping_sub(1); }
                if btn_r { color_hue = color_hue.wrapping_add(1); }

                let t = (frame_count as u16) & 0x1F;
                for y in 0..SCREEN_HEIGHT as u16 {
                    for x in 0..SCREEN_WIDTH as u16 {
                        let r = ((x * 31 / SCREEN_WIDTH as u16) + t) & 0x1F;
                        let g = ((y * 31 / SCREEN_HEIGHT as u16) + (color_hue & 0x1F)) & 0x1F;
                        let b = 15;
                        let bgr555 = r | (g << 5) | (b << 10);
                        let addr = 0x0600_0000 + ((y as u32 * SCREEN_WIDTH as u32 + x as u32) * 2);
                        gba.bus.write16(addr, bgr555);
                    }
                }

                let cursor_color = if btn_a {
                    0x03E0
                } else if btn_b {
                    0x001F
                } else {
                    0x7FFF
                };

                for cy in -4..=4 {
                    for cx in -4..=4 {
                        let px = (cursor_x + cx).clamp(0, SCREEN_WIDTH as i32 - 1) as u32;
                        let py = (cursor_y + cy).clamp(0, SCREEN_HEIGHT as i32 - 1) as u32;
                        let addr = 0x0600_0000 + ((py * SCREEN_WIDTH as u32 + px) * 2);
                        gba.bus.write16(addr, cursor_color);
                    }
                }
            }
        }

        // Jalankan siklus konsol untuk 1 frame (280,896 cycles)
        let mut cycles = 0;
        while cycles < CYCLES_PER_FRAME {
            cycles += gba.step();
        }

        // Alirkan sampel audio APU ke perangkat audio komputer
        let audio_samples = gba.bus.apu.read_samples();
        audio_player.play(&audio_samples);

        // Tampilkan frame buffer ke GUI window
        if let Err(e) = window.update_with_buffer(
            &gba.bus.ppu.frame_buffer[..],
            SCREEN_WIDTH,
            SCREEN_HEIGHT,
        ) {
            eprintln!("Error saat memperbarui window buffer: {}", e);
            break;
        }

        frame_count += 1;

        let elapsed = last_fps_time.elapsed();
        if elapsed >= Duration::from_secs(1) {
            let fps = frame_count as f64 / elapsed.as_secs_f64();
            frame_count = 0;
            last_fps_time = Instant::now();

            let mode_str = match gba.bus.ppu.dispcnt & 7 {
                0 => "Mode 0 (Tilemap)",
                1 => "Mode 1 (Mixed)",
                3 => "Mode 3 (Bitmap)",
                4 => "Mode 4 (Paletted)",
                5 => "Mode 5 (Bitmap)",
                _ => "Other",
            };

            let title = if is_rom_loaded {
                format!(
                    "GBA Emulator - Pure Rust | [{}] | FPS: {:.1} | {}",
                    rom_title_str,
                    fps,
                    mode_str,
                )
            } else {
                format!(
                    "GBA Emulator - Pure Rust [DEMO] | FPS: {:.1} | {} | OBJ: ({}, {})",
                    fps,
                    mode_str,
                    sprite_x,
                    sprite_y,
                )
            };
            window.set_title(&title);
        }
    }

    println!("Emulator ditutup.");
}
