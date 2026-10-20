pub const SCREEN_WIDTH: usize = 240;
pub const SCREEN_HEIGHT: usize = 160;
pub const TOTAL_SCANLINES: usize = 228; // Scanlines 0-159: Draw, 160-227: VBlank

pub const CYCLES_HDRAW: usize = 960;         // 240 piksel x 4 siklus per titik
pub const CYCLES_HBLANK: usize = 272;        // 68 titik x 4 siklus per titik
pub const CYCLES_PER_SCANLINE: usize = 1232; // 308 titik x 4 siklus = 1232 siklus CPU per scanline
pub const CYCLES_PER_FRAME: usize = 280_896; // 1232 x 228 = 280,896 siklus per frame (~59.73 Hz)

/// Event Interrupt yang dihasilkan oleh siklus PPU
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PpuIrqEvents {
    pub vblank: bool,
    pub hblank: bool,
    pub vcount: bool,
}

/// Konversi format warna BGR555 native GBA ke format 32-bit ARGB/XRGB (0xFFRRGGBB)
#[inline(always)]
pub fn bgr555_to_xrgb(val: u16) -> u32 {
    let r = (val & 0x1F) as u32;
    let g = ((val >> 5) & 0x1F) as u32;
    let b = ((val >> 10) & 0x1F) as u32;

    // Skala 5-bit (0..31) ke 8-bit (0..255) secara akurat
    let r8 = (r << 3) | (r >> 2);
    let g8 = (g << 3) | (g >> 2);
    let b8 = (b << 3) | (b >> 2);

    0xFF00_0000 | (r8 << 16) | (g8 << 8) | b8
}

/// Picture Processing Unit (PPU) GBA
pub struct Ppu {
    /// Framebuffer layar RGBA/XRGB berukuran 240x160 piksel
    pub frame_buffer: Box<[u32; SCREEN_WIDTH * SCREEN_HEIGHT]>,

    /// Register DISPCNT (0x0400_0000): Display Control
    pub dispcnt: u16,

    /// Register DISPSTAT (0x0400_0004): Display Status
    pub dispstat: u16,

    /// Register VCOUNT (0x0400_0006): Vertical Counter (scanline 0..=227 saat ini)
    pub scanline: u16,

    /// Register BG0CNT..=BG3CNT (0x0400_0008..=0x0400_000E): Background Control
    pub bgcnt: [u16; 4],

    /// Register BG0HOFS..=BG3HOFS (0x0400_0010, 0x14, 0x18, 0x1C): Horizontal Scroll Offset
    pub bghofs: [u16; 4],

    /// Register BG0VOFS..=BG3VOFS (0x0400_0012, 0x16, 0x1A, 0x1E): Vertical Scroll Offset
    pub bgvofs: [u16; 4],

    /// Akumulator siklus CPU dalam satu scanline saat ini (0..1231)
    pub cycle: usize,
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            frame_buffer: Box::new([0; SCREEN_WIDTH * SCREEN_HEIGHT]),
            dispcnt: 0,
            dispstat: 0,
            scanline: 0,
            bgcnt: [0; 4],
            bghofs: [0; 4],
            bgvofs: [0; 4],
            cycle: 0,
        }
    }

    /// Baca register DISPSTAT secara dinamis berdasarkan scanline dan cycle PPU saat ini
    pub fn read_dispstat(&self) -> u16 {
        let mut stat = self.dispstat & 0xFF38;

        if self.scanline >= 160 && self.scanline < 227 {
            stat |= 1 << 0;
        }

        if self.cycle >= CYCLES_HDRAW {
            stat |= 1 << 1;
        }

        let vcount_setting = (self.dispstat >> 8) & 0xFF;
        if (self.scanline as u32) == vcount_setting as u32 {
            stat |= 1 << 2;
        }

        stat
    }

    /// Tulis ke register DISPSTAT
    pub fn set_dispstat(&mut self, val: u16) {
        let writable_mask = 0xFF38;
        self.dispstat = (self.dispstat & !writable_mask) | (val & writable_mask);
    }

    /// Tulis ke register DISPCNT
    pub fn set_dispcnt(&mut self, val: u16) {
        self.dispcnt = val;
    }

    /// Kembalikan nilai VCOUNT saat ini
    #[inline(always)]
    pub fn vcount(&self) -> u16 {
        self.scanline
    }

    /// Majukan siklus PPU sinkron dengan instruksi CPU dan lakukan rasterisasi scanline
    pub fn step(&mut self, cycles: usize, vram: &[u8], palram: &[u8], oam: &[u8]) -> PpuIrqEvents {
        let mut events = PpuIrqEvents::default();

        let prev_cycle = self.cycle;
        self.cycle += cycles;

        // Transisi masuk ke HBlank (siklus 960)
        if prev_cycle < CYCLES_HDRAW && self.cycle >= CYCLES_HDRAW {
            if (self.dispstat & (1 << 4)) != 0 {
                events.hblank = true;
            }
        }

        // Cek pergantian scanline (setiap 1232 siklus CPU)
        while self.cycle >= CYCLES_PER_SCANLINE {
            self.cycle -= CYCLES_PER_SCANLINE;
            self.scanline = (self.scanline + 1) % (TOTAL_SCANLINES as u16);

            // Rasterisasi scanline pada area yang terlihat (0..=159)
            if self.scanline < (SCREEN_HEIGHT as u16) {
                self.render_scanline(vram, palram, oam);
            }

            // Pemicuan VBlank saat masuk ke scanline 160
            if self.scanline == (SCREEN_HEIGHT as u16) {
                if (self.dispstat & (1 << 3)) != 0 {
                    events.vblank = true;
                }
            }

            // Pemicuan V-Counter Match
            let vcount_setting = ((self.dispstat >> 8) & 0xFF) as u16;
            if self.scanline == vcount_setting {
                if (self.dispstat & (1 << 5)) != 0 {
                    events.vcount = true;
                }
            }
        }

        events
    }

    /// Dimensi sprite (lebar, tinggi) berdasarkan nilai shape dan size
    #[inline(always)]
    pub fn sprite_dimensions(shape: u16, size: u16) -> (usize, usize) {
        match (shape, size) {
            // Square (00b)
            (0, 0) => (8, 8),
            (0, 1) => (16, 16),
            (0, 2) => (32, 32),
            (0, 3) => (64, 64),
            // Horizontal (01b)
            (1, 0) => (16, 8),
            (1, 1) => (32, 8),
            (1, 2) => (32, 16),
            (1, 3) => (64, 32),
            // Vertical (10b)
            (2, 0) => (8, 16),
            (2, 1) => (8, 32),
            (2, 2) => (16, 32),
            (2, 3) => (32, 64),
            _ => (8, 8),
        }
    }

    /// Render seluruh sprite (OBJ) yang aktif pada scanline saat ini ke dalam buffer scanline
    pub fn render_sprites(
        &self,
        y: usize,
        vram: &[u8],
        palram: &[u8],
        oam: &[u8],
        line_buffer: &mut [(u32, u8); SCREEN_WIDTH],
    ) {
        // Cek bit 12 DISPCNT: OBJ Enable
        if (self.dispcnt & (1 << 12)) == 0 {
            return;
        }

        let is_1d_mapping = (self.dispcnt & (1 << 6)) != 0;
        let mode = self.dispcnt & 0x07;

        // Base VRAM offset untuk OBJ tiles:
        // Pada mode teks (0, 1, 2): CharBase 4 = 0x10000
        // Pada mode bitmap (3, 4, 5): CharBase 5 = 0x14000
        let obj_base_offset = if mode >= 3 { 0x14000 } else { 0x10000 };

        // Buffer untuk piksel sprite pada scanline ini: (color, priority, is_drawn)
        let mut obj_buffer: [(u32, u8, bool); SCREEN_WIDTH] = [(0, 4, false); SCREEN_WIDTH];

        // Iterasi 128 sprite dari OAM dalam urutan terbalik (127 down to 0)
        // Objek dengan indeks lebih kecil (0 > 1 > ... > 127) memiliki prioritas lebih tinggi,
        // sehingga menggantikan piksel objek yang indeksnya lebih besar.
        for obj_idx in (0..128).rev() {
            let oam_offset = obj_idx * 8;
            if oam_offset + 8 > oam.len() {
                break;
            }

            let attr0 = u16::from_le_bytes([oam[oam_offset], oam[oam_offset + 1]]);
            let attr1 = u16::from_le_bytes([oam[oam_offset + 2], oam[oam_offset + 3]]);
            let attr2 = u16::from_le_bytes([oam[oam_offset + 4], oam[oam_offset + 5]]);

            let is_affine = (attr0 & (1 << 8)) != 0;
            let double_or_disable = (attr0 & (1 << 9)) != 0;

            // Jika bukan affine dan bit 9 diset, sprite ini di-disable
            if !is_affine && double_or_disable {
                continue;
            }

            let obj_mode = (attr0 >> 10) & 0x03;
            if obj_mode == 3 {
                continue; // Prohibited mode
            }

            let is_8bpp = (attr0 & (1 << 13)) != 0;
            let shape = (attr0 >> 14) & 0x03;
            if shape == 3 {
                continue; // Prohibited shape
            }

            let size = (attr1 >> 14) & 0x03;
            let (w, h) = Self::sprite_dimensions(shape, size);

            let raw_y = (attr0 & 0x00FF) as i32;
            let y_pos = if raw_y >= 160 && raw_y > 256 - (h as i32) {
                raw_y - 256
            } else {
                raw_y
            };

            let raw_x = (attr1 & 0x01FF) as i32;
            let x_pos = if raw_x >= 256 {
                raw_x - 512
            } else {
                raw_x
            };

            let tile_id = (attr2 & 0x03FF) as usize;
            let priority = ((attr2 >> 10) & 0x03) as u8;
            let palette_bank = ((attr2 >> 12) & 0x0F) as usize;

            let cur_y = y as i32;

            if !is_affine {
                // Normal Sprite
                if cur_y < y_pos || cur_y >= y_pos + (h as i32) {
                    continue;
                }

                let h_flip = (attr1 & (1 << 12)) != 0;
                let v_flip = (attr1 & (1 << 13)) != 0;

                let sprite_y = (cur_y - y_pos) as usize;
                let eff_y = if v_flip { h - 1 - sprite_y } else { sprite_y };
                let tile_y = eff_y / 8;
                let py = eff_y % 8;

                for sprite_x in 0..w {
                    let screen_x = x_pos + (sprite_x as i32);
                    if screen_x < 0 || screen_x >= (SCREEN_WIDTH as i32) {
                        continue;
                    }

                    let eff_x = if h_flip { w - 1 - sprite_x } else { sprite_x };
                    let tile_x = eff_x / 8;
                    let px = eff_x % 8;

                    let cur_tile_id = if is_1d_mapping {
                        if is_8bpp {
                            (tile_id + (tile_y * (w / 8) + tile_x) * 2) & 0x03FF
                        } else {
                            (tile_id + tile_y * (w / 8) + tile_x) & 0x03FF
                        }
                    } else {
                        // 2D mapping: baris selebar 32 tile
                        if is_8bpp {
                            (tile_id + tile_y * 32 + tile_x * 2) & 0x03FF
                        } else {
                            (tile_id + tile_y * 32 + tile_x) & 0x03FF
                        }
                    };

                    let color_idx = if is_8bpp {
                        let tile_offset = obj_base_offset + cur_tile_id * 32;
                        let pixel_offset = tile_offset + py * 8 + px;
                        if pixel_offset < vram.len() {
                            vram[pixel_offset]
                        } else {
                            0
                        }
                    } else {
                        let tile_offset = obj_base_offset + cur_tile_id * 32;
                        let row_offset = tile_offset + py * 4;
                        if row_offset + px / 2 < vram.len() {
                            let b = vram[row_offset + px / 2];
                            if px % 2 == 0 { b & 0x0F } else { b >> 4 }
                        } else {
                            0
                        }
                    };

                    // Warna 0 adalah transparan
                    if color_idx != 0 {
                        let pal_addr = if is_8bpp {
                            0x200 + (color_idx as usize) * 2
                        } else {
                            0x200 + palette_bank * 32 + (color_idx as usize) * 2
                        };

                        if pal_addr + 1 < palram.len() {
                            let raw_color = u16::from_le_bytes([palram[pal_addr], palram[pal_addr + 1]]);
                            let color = bgr555_to_xrgb(raw_color);
                            obj_buffer[screen_x as usize] = (color, priority, true);
                        }
                    }
                }
            } else {
                // Affine Sprite
                let double_size = double_or_disable;
                let bb_w = if double_size { w * 2 } else { w };
                let bb_h = if double_size { h * 2 } else { h };

                if cur_y < y_pos || cur_y >= y_pos + (bb_h as i32) {
                    continue;
                }

                let affine_group = ((attr1 >> 9) & 0x1F) as usize;
                let pa_offset = affine_group * 32 + 6;
                let pb_offset = affine_group * 32 + 14;
                let pc_offset = affine_group * 32 + 22;
                let pd_offset = affine_group * 32 + 30;

                if pd_offset + 1 >= oam.len() {
                    continue;
                }

                let pa = i16::from_le_bytes([oam[pa_offset], oam[pa_offset + 1]]) as i32;
                let pb = i16::from_le_bytes([oam[pb_offset], oam[pb_offset + 1]]) as i32;
                let pc = i16::from_le_bytes([oam[pc_offset], oam[pc_offset + 1]]) as i32;
                let pd = i16::from_le_bytes([oam[pd_offset], oam[pd_offset + 1]]) as i32;

                let half_bb_w = (bb_w / 2) as i32;
                let half_bb_h = (bb_h / 2) as i32;
                let half_w = (w / 2) as i32;
                let half_h = (h / 2) as i32;

                let dy = (cur_y - y_pos) - half_bb_h;

                for bb_x in 0..bb_w {
                    let screen_x = x_pos + (bb_x as i32);
                    if screen_x < 0 || screen_x >= (SCREEN_WIDTH as i32) {
                        continue;
                    }

                    let dx = (bb_x as i32) - half_bb_w;

                    // Koordinat tekstur (8.8 fixed point di-shift 8)
                    let tex_x = ((pa * dx + pb * dy) >> 8) + half_w;
                    let tex_y = ((pc * dx + pd * dy) >> 8) + half_h;

                    if tex_x < 0 || tex_x >= (w as i32) || tex_y < 0 || tex_y >= (h as i32) {
                        continue;
                    }

                    let eff_x = tex_x as usize;
                    let eff_y = tex_y as usize;
                    let tile_x = eff_x / 8;
                    let px = eff_x % 8;
                    let tile_y = eff_y / 8;
                    let py = eff_y % 8;

                    let cur_tile_id = if is_1d_mapping {
                        if is_8bpp {
                            (tile_id + (tile_y * (w / 8) + tile_x) * 2) & 0x03FF
                        } else {
                            (tile_id + tile_y * (w / 8) + tile_x) & 0x03FF
                        }
                    } else {
                        if is_8bpp {
                            (tile_id + tile_y * 32 + tile_x * 2) & 0x03FF
                        } else {
                            (tile_id + tile_y * 32 + tile_x) & 0x03FF
                        }
                    };

                    let color_idx = if is_8bpp {
                        let tile_offset = obj_base_offset + cur_tile_id * 32;
                        let pixel_offset = tile_offset + py * 8 + px;
                        if pixel_offset < vram.len() {
                            vram[pixel_offset]
                        } else {
                            0
                        }
                    } else {
                        let tile_offset = obj_base_offset + cur_tile_id * 32;
                        let row_offset = tile_offset + py * 4;
                        if row_offset + px / 2 < vram.len() {
                            let b = vram[row_offset + px / 2];
                            if px % 2 == 0 { b & 0x0F } else { b >> 4 }
                        } else {
                            0
                        }
                    };

                    if color_idx != 0 {
                        let pal_addr = if is_8bpp {
                            0x200 + (color_idx as usize) * 2
                        } else {
                            0x200 + palette_bank * 32 + (color_idx as usize) * 2
                        };

                        if pal_addr + 1 < palram.len() {
                            let raw_color = u16::from_le_bytes([palram[pal_addr], palram[pal_addr + 1]]);
                            let color = bgr555_to_xrgb(raw_color);
                            obj_buffer[screen_x as usize] = (color, priority, true);
                        }
                    }
                }
            }
        }

        // Priority mixing: Gabungkan piksel sprite ke line_buffer
        // Jika prioritas sprite <= prioritas background, sprite tampil di depan
        for x in 0..SCREEN_WIDTH {
            let (obj_color, obj_prio, is_drawn) = obj_buffer[x];
            if is_drawn {
                let (_, bg_prio) = line_buffer[x];
                if obj_prio <= bg_prio {
                    line_buffer[x] = (obj_color, obj_prio);
                }
            }
        }
    }

    /// Rasterisasi satu baris horizontal (scanline) ke dalam frame_buffer
    pub fn render_scanline(&mut self, vram: &[u8], palram: &[u8], oam: &[u8]) {
        let y = self.scanline as usize;
        if y >= SCREEN_HEIGHT {
            return;
        }

        let line_start = y * SCREEN_WIDTH;

        // Forced Blank (Bit 7 DISPCNT): Layar menjadi putih solid
        if (self.dispcnt & (1 << 7)) != 0 {
            self.frame_buffer[line_start..line_start + SCREEN_WIDTH].fill(0xFFFF_FFFF);
            return;
        }

        let mode = self.dispcnt & 0x7;
        let frame_select = (self.dispcnt & (1 << 4)) != 0;

        // Warna latar belakang (Backdrop Color: Palette RAM index 0)
        let backdrop_raw = if palram.len() >= 2 {
            u16::from_le_bytes([palram[0], palram[1]])
        } else {
            0
        };
        let backdrop_color = bgr555_to_xrgb(backdrop_raw);

        // Inisialisasi line_buffer dengan backdrop color pada prioritas terendah (4)
        let mut line_buffer = [(backdrop_color, 4u8); SCREEN_WIDTH];

        match mode {
            // Mode 0: 4 Text Backgrounds (BG0-BG3) dengan prioritas dan scrolling
            0 => {
                for bg_num in (0..4).rev() {
                    let bg_enable = (self.dispcnt & (1 << (8 + bg_num))) != 0;
                    if bg_enable {
                        self.render_text_bg(bg_num, y, vram, palram, &mut line_buffer);
                    }
                }
            }

            // Mode 1: BG0 (Text), BG1 (Text), BG2 (Affine)
            1 => {
                for bg_num in (0..2).rev() {
                    let bg_enable = (self.dispcnt & (1 << (8 + bg_num))) != 0;
                    if bg_enable {
                        self.render_text_bg(bg_num, y, vram, palram, &mut line_buffer);
                    }
                }
            }

            // Mode 3: 240x160 16-bit direct color BGR555
            3 => {
                let bg2_enable = (self.dispcnt & (1 << 10)) != 0;
                if bg2_enable {
                    let vram_offset = y * SCREEN_WIDTH * 2;
                    for x in 0..SCREEN_WIDTH {
                        let offset = vram_offset + x * 2;
                        if offset + 1 < vram.len() {
                            let raw = u16::from_le_bytes([vram[offset], vram[offset + 1]]);
                            line_buffer[x] = (bgr555_to_xrgb(raw), 2);
                        }
                    }
                }
            }

            // Mode 4: 240x160 8-bit palette-indexed dengan page flipping (Frame 0/1)
            4 => {
                let bg2_enable = (self.dispcnt & (1 << 10)) != 0;
                if bg2_enable {
                    let frame_base = if frame_select { 0xA000 } else { 0x0000 };
                    let vram_offset = frame_base + y * SCREEN_WIDTH;

                    for x in 0..SCREEN_WIDTH {
                        let offset = vram_offset + x;
                        if offset < vram.len() {
                            let pal_idx = vram[offset] as usize;
                            if pal_idx != 0 {
                                let pal_offset = pal_idx * 2;
                                if pal_offset + 1 < palram.len() {
                                    let raw = u16::from_le_bytes([palram[pal_offset], palram[pal_offset + 1]]);
                                    line_buffer[x] = (bgr555_to_xrgb(raw), 2);
                                }
                            }
                        }
                    }
                }
            }

            // Mode 5: 160x128 16-bit direct color dengan page flipping
            5 => {
                let bg2_enable = (self.dispcnt & (1 << 10)) != 0;
                if bg2_enable && y < 128 {
                    let frame_base = if frame_select { 0xA000 } else { 0x0000 };
                    let vram_offset = frame_base + y * 160 * 2;

                    for x in 0..160 {
                        let offset = vram_offset + x * 2;
                        if offset + 1 < vram.len() {
                            let raw = u16::from_le_bytes([vram[offset], vram[offset + 1]]);
                            line_buffer[x] = (bgr555_to_xrgb(raw), 2);
                        }
                    }
                }
            }

            _ => {}
        }

        // Render sprite dan gabungkan dengan background
        self.render_sprites(y, vram, palram, oam, &mut line_buffer);

        for x in 0..SCREEN_WIDTH {
            self.frame_buffer[line_start + x] = line_buffer[x].0;
        }
    }

    /// Render satu layer Text Background (BG0-BG3) ke dalam scanline buffer sementara
    fn render_text_bg(
        &self,
        bg_num: usize,
        y: usize,
        vram: &[u8],
        palram: &[u8],
        line_buffer: &mut [(u32, u8); SCREEN_WIDTH],
    ) {
        let bgcnt = self.bgcnt[bg_num];
        let priority = (bgcnt & 0x3) as u8;
        let char_base = ((bgcnt >> 2) & 0x3) as usize * 16 * 1024;
        let is_8bpp = (bgcnt & (1 << 7)) != 0;
        let screen_base = ((bgcnt >> 8) & 0x1F) as usize * 2 * 1024;
        let screen_size = (bgcnt >> 14) & 0x3;

        let hofs = (self.bghofs[bg_num] & 0x01FF) as usize;
        let vofs = (self.bgvofs[bg_num] & 0x01FF) as usize;

        // Ukuran virtual map dalam piksel berdasarkan bits 14..=15 BGnCNT
        let (map_width, map_height) = match screen_size {
            0 => (256, 256), // 32x32 tiles (1 screenblock)
            1 => (512, 256), // 64x32 tiles (2 screenblocks horizontal)
            2 => (256, 512), // 32x64 tiles (2 screenblocks vertikal)
            3 => (512, 512), // 64x64 tiles (4 screenblocks 2x2)
            _ => unreachable!(),
        };

        let map_y = (y + vofs) % map_height;

        for x in 0..SCREEN_WIDTH {
            let map_x = (x + hofs) % map_width;

            // Hitung screenblock index (0..3) dan koordinat lokal di dalam screenblock (0..255)
            let (sb_idx, local_x, local_y) = match screen_size {
                0 => (0, map_x, map_y),
                1 => {
                    if map_x >= 256 { (1, map_x - 256, map_y) } else { (0, map_x, map_y) }
                }
                2 => {
                    if map_y >= 256 { (1, map_x, map_y - 256) } else { (0, map_x, map_y) }
                }
                3 => {
                    let sb_x = if map_x >= 256 { 1 } else { 0 };
                    let sb_y = if map_y >= 256 { 1 } else { 0 };
                    let lx = if map_x >= 256 { map_x - 256 } else { map_x };
                    let ly = if map_y >= 256 { map_y - 256 } else { map_y };
                    (sb_y * 2 + sb_x, lx, ly)
                }
                _ => unreachable!(),
            };

            // Satu screenblock berukuran 2048 bytes (32x32 entri tile, masing-masing 2 bytes)
            let sb_addr = screen_base + sb_idx * 2048;
            let tx = local_x / 8;
            let ty = local_y / 8;
            let entry_offset = sb_addr + (ty * 32 + tx) * 2;

            if entry_offset + 1 >= vram.len() {
                continue;
            }

            let tile_entry = u16::from_le_bytes([vram[entry_offset], vram[entry_offset + 1]]);
            let tile_num = (tile_entry & 0x3FF) as usize;
            let h_flip = (tile_entry & (1 << 10)) != 0;
            let v_flip = (tile_entry & (1 << 11)) != 0;
            let pal_bank = ((tile_entry >> 12) & 0xF) as usize;

            let mut px = local_x % 8;
            let mut py = local_y % 8;

            if h_flip { px = 7 - px; }
            if v_flip { py = 7 - py; }

            let (pal_idx, color_offset) = if is_8bpp {
                // 8bpp Mode: 64 bytes per tile (1 byte per piksel)
                let tile_offset = char_base + tile_num * 64 + py * 8 + px;
                if tile_offset >= vram.len() {
                    continue;
                }
                let idx = vram[tile_offset] as usize;
                (idx, idx * 2)
            } else {
                // 4bpp Mode: 32 bytes per tile (4 bytes per baris, 2 piksel per byte)
                let tile_offset = char_base + tile_num * 32 + py * 4 + (px / 2);
                if tile_offset >= vram.len() {
                    continue;
                }
                let byte = vram[tile_offset];
                let idx = if (px & 1) == 0 {
                    (byte & 0x0F) as usize
                } else {
                    ((byte >> 4) & 0x0F) as usize
                };
                (idx, (pal_bank * 16 + idx) * 2)
            };

            // Index 0 adalah transparan untuk tile layer
            if pal_idx != 0 && color_offset + 1 < palram.len() {
                // Priority test: layer dengan prioritas lebih tinggi (angka lebih kecil atau sama) menang
                if priority <= line_buffer[x].1 {
                    let color_raw = u16::from_le_bytes([palram[color_offset], palram[color_offset + 1]]);
                    line_buffer[x] = (bgr555_to_xrgb(color_raw), priority);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bgr555_to_xrgb_conversion() {
        assert_eq!(bgr555_to_xrgb(0), 0xFF00_0000);
        assert_eq!(bgr555_to_xrgb(0x001F), 0xFFFF_0000); // Merah
        assert_eq!(bgr555_to_xrgb(0x03E0), 0xFF00_FF00); // Hijau
        assert_eq!(bgr555_to_xrgb(0x7C00), 0xFF00_00FF); // Biru
        assert_eq!(bgr555_to_xrgb(0x7FFF), 0xFFFF_FFFF); // Putih
    }

    #[test]
    fn test_ppu_scanline_stepping_and_vblank_irq() {
        let mut ppu = Ppu::new();
        let vram = vec![0u8; 96 * 1024];
        let palram = vec![0u8; 1024];
        let oam = vec![0u8; 1024];

        ppu.set_dispstat(1 << 3);

        for _ in 0..159 {
            let events = ppu.step(CYCLES_PER_SCANLINE, &vram, &palram, &oam);
            assert!(!events.vblank);
        }

        let events = ppu.step(CYCLES_PER_SCANLINE, &vram, &palram, &oam);
        assert!(events.vblank);
        assert_eq!(ppu.scanline, 160);
        assert!((ppu.read_dispstat() & (1 << 0)) != 0);
    }

    #[test]
    fn test_mode3_direct_color_rendering() {
        let mut ppu = Ppu::new();
        let mut vram = vec![0u8; 96 * 1024];
        let palram = vec![0u8; 1024];
        let oam = vec![0u8; 1024];

        ppu.set_dispcnt(0x0403);
        vram[0] = 0x1F;
        vram[1] = 0x00;

        ppu.scanline = 0;
        ppu.render_scanline(&vram, &palram, &oam);

        assert_eq!(ppu.frame_buffer[0], 0xFFFF_0000);
    }

    #[test]
    fn test_mode0_4bpp_tile_rendering() {
        let mut ppu = Ppu::new();
        let mut vram = vec![0u8; 96 * 1024];
        let mut palram = vec![0u8; 1024];
        let oam = vec![0u8; 1024];

        // 1. Set DISPCNT: Mode 0 (0b000) + Enable BG0 (bit 8 = 0x0100) -> 0x0100
        ppu.set_dispcnt(0x0100);

        // 2. Set BG0CNT: CharBase 0 (offset 0), ScreenBase 2 (offset 2 * 2KB = 4096 = 0x1000), 4bpp, Priority 0
        ppu.bgcnt[0] = 0x0200;

        // 3. Buat tile 0 pada CharBase 0 (offset 0):
        vram[0] = 0x01; // Nibble rendah = 1 (piksel 0), nibble tinggi = 0 (piksel 1)

        // 4. Buat entri tilemap pada ScreenBase 2 (offset 0x1000):
        vram[0x1000] = 0x00;
        vram[0x1001] = 0x00;

        // 5. Set Palette RAM:
        // Palette bank 0, indeks 1 -> offset 2. Warna biru: 0x7C00
        palram[2] = 0x00;
        palram[3] = 0x7C;

        ppu.scanline = 0;
        ppu.render_scanline(&vram, &palram, &oam);

        // Piksel (0, 0) harus berwarna biru ARGB 32-bit (0xFF0000FF)
        assert_eq!(ppu.frame_buffer[0], 0xFF00_00FF);
        // Piksel (1, 0) adalah transparan (indeks 0), harus menjadi backdrop color
        assert_eq!(ppu.frame_buffer[1], 0xFF00_0000);
    }

    #[test]
    fn test_mode0_hflip_tile() {
        let mut ppu = Ppu::new();
        let mut vram = vec![0u8; 96 * 1024];
        let mut palram = vec![0u8; 1024];
        let oam = vec![0u8; 1024];

        ppu.set_dispcnt(0x0100);
        ppu.bgcnt[0] = 0x0200;

        // Tile 0: piksel (0, 0) = 1
        vram[0] = 0x01;

        // Tilemap entry di 0x1000: Tile 0 dengan H-Flip aktif (bit 10 = 0x0400)
        vram[0x1000] = 0x00;
        vram[0x1001] = 0x04;

        // Warna hijau: 0x03E0 di indeks palette 1
        palram[2] = 0xE0;
        palram[3] = 0x03;

        ppu.scanline = 0;
        ppu.render_scanline(&vram, &palram, &oam);

        // Karena di-flip horizontal, piksel (0, 0) bergeser ke piksel (7, 0)!
        assert_eq!(ppu.frame_buffer[0], 0xFF00_0000); // Bukan hijau
        assert_eq!(ppu.frame_buffer[7], 0xFF00_FF00); // Hijau!
    }

    #[test]
    fn test_mode0_scrolling_hofs() {
        let mut ppu = Ppu::new();
        let mut vram = vec![0u8; 96 * 1024];
        let mut palram = vec![0u8; 1024];
        let oam = vec![0u8; 1024];

        ppu.set_dispcnt(0x0100);
        ppu.bgcnt[0] = 0x0200;

        // Tile 0: piksel (0, 0) = 1 (warna merah 0x001F)
        vram[0] = 0x01;
        palram[2] = 0x1F;
        palram[3] = 0x00;

        // Tilemap entry 0x1000: Tile 0
        vram[0x1000] = 0x00;
        vram[0x1001] = 0x00;

        // Set BG0HOFS = 0
        ppu.bghofs[0] = 0;
        ppu.scanline = 0;
        ppu.render_scanline(&vram, &palram, &oam);
        assert_eq!(ppu.frame_buffer[0], 0xFFFF_0000); // Merah di x=0

        // Sekarang set BG0HOFS = 2 (scroll ke kanan 2 piksel di virtual map)
        ppu.bghofs[0] = 2;
        ppu.render_scanline(&vram, &palram, &oam);
        assert_eq!(ppu.frame_buffer[0], 0xFF00_0000); // Di screen x=0 bukan lagi merah
    }

    #[test]
    fn test_mode0_priority_mixing() {
        let mut ppu = Ppu::new();
        let mut vram = vec![0u8; 96 * 1024];
        let mut palram = vec![0u8; 1024];
        let oam = vec![0u8; 1024];

        // Enable BG0 dan BG1 pada DISPCNT (bits 8 & 9) -> 0x0300
        ppu.set_dispcnt(0x0300);

        // BG0: Priority 2 (belakang), ScreenBase 1 (0x0800), CharBase 0
        ppu.bgcnt[0] = 0x0102;
        // BG1: Priority 1 (depan), ScreenBase 2 (0x1000), CharBase 0
        ppu.bgcnt[1] = 0x0201;

        // Tile 0: piksel (0, 0) = indeks 1 (Merah: 0x001F)
        vram[0] = 0x01;
        // Tile 1: piksel (0, 0) = indeks 2 (Hijau: 0x03E0)
        vram[32] = 0x02;

        palram[2] = 0x1F; // Palette 1 = Merah
        palram[3] = 0x00;
        palram[4] = 0xE0; // Palette 2 = Hijau
        palram[5] = 0x03;

        // BG0 menaruh Tile 0 di (0,0)
        vram[0x0800] = 0x00;
        vram[0x0801] = 0x00;

        // BG1 menaruh Tile 1 di (0,0)
        vram[0x1000] = 0x01;
        vram[0x1001] = 0x00;

        ppu.scanline = 0;
        ppu.render_scanline(&vram, &palram, &oam);

        assert_eq!(ppu.frame_buffer[0], 0xFF00_FF00);
    }

    #[test]
    fn test_sprite_4bpp_normal_rendering() {
        let mut ppu = Ppu::new();
        let mut vram = vec![0u8; 96 * 1024];
        let mut palram = vec![0u8; 1024];
        let mut oam = vec![0u8; 1024];

        // Enable OBJ (bit 12 = 0x1000)
        ppu.set_dispcnt(0x1000);

        // Setup OBJ 0 di OAM:
        // attr0: Y = 10, Normal (bit 8 = 0), Not disabled (bit 9 = 0), 4bpp (bit 13 = 0), Square (00) -> 0x000A
        oam[0] = 0x0A;
        oam[1] = 0x00;
        // attr1: X = 20, No flip, Size 0 (8x8) -> 0x0014
        oam[2] = 0x14;
        oam[3] = 0x00;
        // attr2: Tile 0, Priority 0, Palette 0 -> 0x0000
        oam[4] = 0x00;
        oam[5] = 0x00;

        // OBJ tile 0 di CharBase 4 (offset 0x10000):
        // Piksel (0, 0) = indeks warna 1
        vram[0x10000] = 0x01;

        // Sprite Palette 0, indeks 1 di 0x0500_0202 (offset 0x202):
        // Warna Kuning (0x03FF)
        palram[0x202] = 0xFF;
        palram[0x203] = 0x03;

        // Render scanline 10 (baris pertama sprite)
        ppu.scanline = 10;
        ppu.render_scanline(&vram, &palram, &oam);

        // Piksel di layar X = 20 harus berwarna Kuning ARGB (0xFFFFFF00)
        assert_eq!(ppu.frame_buffer[10 * SCREEN_WIDTH + 20], 0xFFFF_FF00);
        // Piksel di luar sprite X = 19 harus backdrop (hitam)
        assert_eq!(ppu.frame_buffer[10 * SCREEN_WIDTH + 19], 0xFF00_0000);
    }

    #[test]
    fn test_sprite_priority_over_background() {
        let mut ppu = Ppu::new();
        let mut vram = vec![0u8; 96 * 1024];
        let mut palram = vec![0u8; 1024];
        let mut oam = vec![0u8; 1024];

        // Enable BG0 (bit 8) + Enable OBJ (bit 12) -> 0x1100
        ppu.set_dispcnt(0x1100);

        // BG0: Priority 1, CharBase 0, ScreenBase 2
        ppu.bgcnt[0] = 0x0201; // Priority 1

        // BG Tile 0 piksel (0, 0) = indeks 1 (Merah: 0x001F)
        vram[0] = 0x01;
        palram[2] = 0x1F;
        palram[3] = 0x00;
        vram[0x1000] = 0x00; // BG0 tilemap di (0,0)

        // OBJ 0 di (X=0, Y=0): Priority 1 (sama dengan BG0!).
        // Sesuai aturan hardware GBA: Jika prioritas sama (OBJ Priority == BG Priority),
        // OBJ menang dan ditampilkan di depan!
        oam[0] = 0x00; // Y = 0
        oam[1] = 0x00;
        oam[2] = 0x00; // X = 0
        oam[3] = 0x00;
        oam[4] = 0x00; // Tile 0, Priority 1 (bits 10-11 = 01b -> 0x0400)
        oam[5] = 0x04;

        // OBJ Tile 0 piksel (0, 0) = indeks 1 (Hijau: 0x03E0)
        vram[0x10000] = 0x01;
        palram[0x202] = 0xE0;
        palram[0x203] = 0x03;

        ppu.scanline = 0;
        ppu.render_scanline(&vram, &palram, &oam);

        // Piksel (0, 0) harus berwarna HIJAU dari OBJ!
        assert_eq!(ppu.frame_buffer[0], 0xFF00_FF00);
    }

    #[test]
    fn test_sprite_horizontal_flip() {
        let mut ppu = Ppu::new();
        let mut vram = vec![0u8; 96 * 1024];
        let mut palram = vec![0u8; 1024];
        let mut oam = vec![0u8; 1024];

        // Nonaktifkan semua sprite di OAM (bit 9 attr0 = 1 untuk disable)
        for i in 0..128 {
            oam[i * 8 + 1] = 0x02;
        }

        ppu.set_dispcnt(0x1000); // Enable OBJ

        // OBJ 0: Y=0, X=0, H-Flip aktif (bit 12 = 0x1000 pada attr1)
        oam[0] = 0x00;
        oam[1] = 0x00; // Enable (bit 9 = 0)
        oam[2] = 0x00;
        oam[3] = 0x10; // H-Flip
        oam[4] = 0x00;
        oam[5] = 0x00;

        // Piksel (0, 0) di tile adalah indeks 1 (Cyan: 0x7FE0)
        vram[0x10000] = 0x01;
        palram[0x202] = 0xE0;
        palram[0x203] = 0x7F;

        ppu.scanline = 0;
        ppu.render_scanline(&vram, &palram, &oam);

        // Dengan H-Flip, piksel x=0 bergeser ke x=7
        assert_eq!(ppu.frame_buffer[0], 0xFF00_0000); // Bukan cyan
        assert_eq!(ppu.frame_buffer[7], 0xFF00_FFFF); // Cyan di x=7!
    }
}
