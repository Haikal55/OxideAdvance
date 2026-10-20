# OxideAdvance 🦀

[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Tests](https://img.shields.io/badge/Tests-49%20Passed-brightgreen.svg)]()
[![Core](https://img.shields.io/badge/Core-Pure%20Rust%20(Zero%20C%20Deps)-success.svg)]()
[![Target](https://img.shields.io/badge/Performance-60%20FPS%20Realtime-brightgreen.svg)]()

**OxideAdvance** adalah emulator Game Boy Advance (GBA) modular berkinerja tinggi yang dibangun dari awal (*from scratch*) menggunakan **Pure Rust** tanpa ketergantungan library C eksternal pada core emulasinya.

Emulator ini mengimplementasikan emulasi siklus waktu (*cycle-synchronized*) akurat untuk CPU ARM7TDMI (ARMv4T 32-bit & THUMB 16-bit dengan 3-stage pipeline), Unified Memory Bus (MMU dengan mirroring), PPU (Tile Modes 0-2 & Bitmap Modes 3-5 dengan Sprite OAM), DMA Channels 0-3, Programmable Timers 0-3, Interrupt Controller, APU (PSG Synthesizer + DirectSound FIFO Stereo), dan Cleanroom HLE BIOS.

---

### 🎮 Commercial Game Compatibility Showcase

OxideAdvance mampu menjalankan game komersial GBA secara *out-of-the-box* (contoh: **Pokemon FireRed Version**):

| 1. Shooting Stars Intro | 2. Game Freak Logo | 3. Gengar Battle Intro |
| :---: | :---: | :---: |
| ![Pokemon Stars Intro](docs/screenshots/pokemon_stars.png) | ![Game Freak Logo](docs/screenshots/gamefreak_logo.png) | ![Pokemon Gengar Intro](docs/screenshots/pokemon_gengar_intro.png) |

---

## Daftar Isi

1. [Arsitektur Sistem](#arsitektur-sistem)
2. [Struktur Workspace](#struktur-workspace)
3. [Rincian Subsistem Hardware](#rincian-subsistem-hardware)
   - [ARM7TDMI CPU Core](#1-arm7tdmi-cpu-core)
   - [Memory Bus & Memory Map](#2-memory-bus--memory-map)
   - [PPU & Graphics Engine](#3-ppu--graphics-engine)
   - [DMA Controller](#4-dma-controller)
   - [Programmable Timers](#5-programmable-timers)
   - [Audio Processing Unit (APU)](#6-audio-processing-unit-apu)
   - [Interrupt Controller](#7-interrupt-controller)
   - [Keypad Controller](#8-keypad-controller)
4. [Siklus Sinkronisasi Emulasi (Cycle-Stepping)](#siklus-sinkronisasi-emulasi-cycle-stepping)
5. [Peta Register I/O GBA](#peta-register-io-gba)
6. [Kontrol Keyboard Desktop](#kontrol-keyboard-desktop)
7. [Cara Kompilasi & Menjalankan](#cara-kompilasi--menjalankan)
8. [Pengujian & Verifikasi](#pengujian--verifikasi)
9. [Lisensi](#lisensi)

---

## Arsitektur Sistem

Seluruh sistem dirancang secara terdekopel di mana `GbaBus` bertindak sebagai bus bersama yang menghubungkan CPU, PPU, DMA, Timer, Interrupts, dan APU tanpa *borrow conflicts* dan tanpa `unsafe`.

```mermaid
flowchart TD
    subgraph CPU_SUB["CPU Subsystem"]
        CPU["ARM7TDMI Core<br/>ARMv4T 32-bit & THUMB 16-bit"]
        Pipe["3-Stage Pipeline<br/>Fetch - Decode - Execute"]
        Regs["Register Banking<br/>R0-R15 + CPSR/SPSR"]
        CPU --> Pipe
        CPU --> Regs
    end

    subgraph BUS_SUB["Unified Memory Bus (GbaBus)"]
        Bus["GbaBus Controller"]
        BIOS["BIOS (16 KB)"]
        EWRAM["EWRAM (256 KB)"]
        IWRAM["IWRAM (32 KB Fast)"]
        VRAM["VRAM (96 KB)"]
        PALRAM["PALRAM (1 KB)"]
        OAM["OAM (1 KB)"]
        ROM["Game Pak ROM (up to 32 MB)"]
        Bus --- BIOS
        Bus --- EWRAM
        Bus --- IWRAM
        Bus --- VRAM
        Bus --- PALRAM
        Bus --- OAM
        Bus --- ROM
    end

    subgraph PERIPH_SUB["Peripherals Subsystem"]
        PPU["PPU Rasterizer<br/>Modes 0-5 + Sprites"]
        DMA["DMA Controller<br/>Channels 0-3"]
        Timers["Timers 0-3<br/>Prescaler & Cascade"]
        APU["APU Sound Engine<br/>PSG + DirectSound A/B"]
        Interrupts["Interrupt Controller<br/>IE / IF / IME"]
        Keypad["Keypad Controller<br/>Active-Low KEYINPUT"]
    end

    subgraph GUI_SUB["Desktop Frontend"]
        GUI["minifb Window 60 FPS<br/>4x Scaled (960x640)"]
    end

    CPU <-->|"8, 16, 32-bit Access"| Bus
    Bus <--> PPU
    Bus <--> DMA
    Bus <--> Timers
    Bus <--> APU
    Bus <--> Interrupts
    Bus <--> Keypad

    PPU -.->|"VBlank / HBlank"| DMA
    PPU -.->|"Display IRQ"| Interrupts
    Timers -.->|"Clock & Overflows"| APU
    Timers -.->|"Timer IRQ"| Interrupts
    APU -.->|"DMA Request"| DMA
    Interrupts -.->|"IRQ Signal"| CPU

    PPU -->|"XRGB Frame Buffer"| GUI
    GUI -->|"Key Events"| Keypad
```

---

## Struktur Workspace

Proyek ini menggunakan Cargo Workspace yang memisahkan logika inti emulasi dari layer I/O antarmuka pengguna:

```text
.
├── Cargo.toml                       # Root Workspace Configuration
├── crates
│   ├── gba-core                     # Core Emulasi GBA (Pure Rust, no_std friendly, Zero C deps)
│   │   ├── Cargo.toml
│   │   └── src
│   │       ├── lib.rs               # Library root & module exports
│   │       ├── gba.rs               # Master GBA System Coordinator
│   │       ├── cpu/                 # ARM7TDMI Core
│   │       │   ├── mod.rs           # Arm7Tdmi struct, pipeline, & IRQ dispatch
│   │       │   ├── cpsr.rs          # CPSR/SPSR status flags & condition evaluator
│   │       │   ├── registers.rs     # Banked register storage (User, FIQ, IRQ, SVC, etc.)
│   │       │   ├── alu.rs           # Barrel Shifter & 16 ALU Operations
│   │       │   ├── arm/             # 32-bit ARM instruction decoder & executor
│   │       │   └── thumb/           # 16-bit THUMB instruction decoder (Formats 1-19)
│   │       ├── bus/                 # Memory Bus & Memory Map
│   │       │   ├── mod.rs           # Bus Trait & Unaligned access logic
│   │       │   └── memory.rs        # GbaBus, mirroring, & I/O dispatch
│   │       ├── ppu/                 # Picture Processing Unit
│   │       │   └── mod.rs           # Scanline rasterizer, Modes 0-5, Sprites/OBJ
│   │       ├── dma/                 # Direct Memory Access
│   │       │   └── mod.rs           # DMA Channels 0-3 & timing triggers
│   │       ├── timers/              # Timers
│   │       │   └── mod.rs           # Timers 0-3, Prescalers, & Cascade mode
│   │       ├── apu/                 # Audio Processing Unit
│   │       │   └── mod.rs           # DirectSound FIFO A/B, PSG Ch1, & Stereo Mixer
│   │       ├── interrupts/          # Interrupt Controller
│   │       │   └── mod.rs           # IE, IF, IME registers & IRQ masking
│   │       └── keypad/              # Keypad Controller
│   │           └── mod.rs           # KEYINPUT (active-low) & KEYCNT
│   └── gba-desktop                  # Native Desktop GUI & CLI Launcher
│       ├── Cargo.toml
│       └── src
│           └── main.rs              # minifb GUI window, 60 FPS loop, & interactive demo
├── docs
│   └── screenshots/                 # Tangkapan layar emulasi visual (Pokemon FireRed)
└── README.md
```

---

## Rincian Subsistem Hardware

### 1. ARM7TDMI CPU Core

ARM7TDMI merupakan mikroprosesor RISC 32-bit yang mendukung dua set instruksi: **ARMv4T** (32-bit) dan **THUMB** (16-bit kompak).

```mermaid
flowchart TD
    subgraph PIPELINE["3-Stage Instruction Pipeline"]
        F["1. Fetch Stage (Ambil instruksi dari Bus)"]
        D["2. Decode Stage (Decode instruksi ke kontrol kontrol)"]
        E["3. Execute Stage (Eksekusi ALU / Branch / Load-Store)"]
        F --> D --> E
    end

    subgraph BANKING["Register Bank Management"]
        R_ACT["Active Registers: R0 - R15 (PC)"]
        R_CPSR["CPSR (Flags: N, Z, C, V, I, F, T, Mode)"]
        R_SPSR["Saved SPSR (FIQ, IRQ, SVC, ABT, UND)"]
        R_BANK["Banked R13 (SP) & R14 (LR) per Mode"]
        R_FIQ["Banked R8 - R12 (FIQ Mode)"]
        R_ACT --- R_CPSR
        R_ACT --- R_BANK
        R_ACT --- R_FIQ
        R_CPSR --- R_SPSR
    end

    PIPELINE <--> BANKING
```

#### Fitur & Karakteristik Utama CPU:
- **3-Stage Pipeline (Fetch-Decode-Execute)**:
  - Disimulasikan dengan buffer `pipeline: [u32; 2]`.
  - Pada arsitektur ARM7TDMI, saat instruksi dieksekusi, nilai register `PC` (R15) terbaca **8 byte di depan** (pada mode ARM) atau **4 byte di depan** (pada mode THUMB).
  - Melakukan *pipeline refill* saat terjadi branch, branch with link, mode exchange (BX), atau exception.
- **Register Banking**:
  - Menyimpan register aktif dalam array flat `r: [u32; 16]`.
  - Pergantian mode CPU (`User`, `FIQ`, `IRQ`, `Supervisor`, `Abort`, `Undefined`, `System`) menukar register banked (`R13_sp`, `R14_lr`, dan `R8_fiq..R12_fiq`) secara branchless O(1).
- **ALU & Barrel Shifter**:
  - Implementasi 16 operasi ALU ARM (`AND`, `EOR`, `SUB`, `RSB`, `ADD`, `ADC`, `SBC`, `RSC`, `TST`, `TEQ`, `CMP`, `CMN`, `ORR`, `MOV`, `BIC`, `MVN`).
  - Mendukung 5 tipe shift: LSL (Logical Shift Left), LSR (Logical Shift Right), ASR (Arithmetic Shift Right), ROR (Rotate Right), dan RRX (Rotate Right Extended with Carry).
  - Penanganan aturan Carry ARM pada operasi pengurangan: Carry flag bertindak sebagai **Not-Borrow** (C=1 jika a >= b, 0 jika terjadi pinjaman/borrow).
- **Decoder Lengkap**:
  - Set instruksi ARM 32-bit: Data Processing, Multiply/Multiply-Accumulate, Single Data Transfer (LDR/STR), Halfword/Signed Transfer (LDRH/STRH/LDRSB/LDRSH), Block Data Transfer (LDM/STM dalam mode IA/IB/DA/DB dengan pemulihan CPSR dan transfer bank User), Branch & Link (B/BL), Branch Exchange (BX), Status Register Access (MRS/MSR), dan Software Interrupt (SWI).
  - Set instruksi THUMB 16-bit: Formats 1 hingga 19 lengkap mencakup ALU hi-register, PC-relative load, push/pop dengan reload PC/LR, dan unconditional/conditional branches.

---

### 2. Memory Bus & Memory Map

Bus memori memetakan ruang alamat fisik 32-bit GBA (`0x0000_0000..=0x0FFF_FFFF`) dengan perlakuan hardware khusus.

| Rentang Alamat | Ukuran | Deskripsi & Hak Akses | Karakteristik Hardware |
| :--- | :--- | :--- | :--- |
| `0x0000_0000..=0x0000_3FFF` | 16 KB | System ROM / BIOS | Read-Only. Cleanroom default BIOS & Exception vectors |
| `0x0200_0000..=0x0203_FFFF` | 256 KB | On-board WRAM (EWRAM) | Read/Write 16-bit bus (mirrored hingga `0x02FF_FFFF`) |
| `0x0300_0000..=0x0300_7FFF` | 32 KB | On-chip WRAM (IWRAM) | Read/Write 32-bit bus super cepat (mirrored hingga `0x03FF_FFFF`) |
| `0x0400_0000..=0x0400_03FF` | 1 KB | I/O Registers | Register hardware PPU, DMA, Timer, APU, Keypad |
| `0x0500_0000..=0x0500_03FF` | 1 KB | Palette RAM (PALRAM) | 512 byte BG palette + 512 byte OBJ palette |
| `0x0600_0000..=0x0601_7FFF` | 96 KB | Video RAM (VRAM) | BG tilemaps, character data, bitmap framebuffers |
| `0x0700_0000..=0x0700_03FF` | 1 KB | Object Attribute Memory (OAM) | 128 entri atribut sprite + 32 set matriks affine |
| `0x0800_0000..=0x09FF_FFFF` | Max 32 MB | Game Pak ROM (Waitstate 0) | Akses kartrid ROM game |
| `0x0A00_0000..=0x0BFF_FFFF` | Max 32 MB | Game Pak ROM (Waitstate 1) | Mirror Waitstate 1 |
| `0x0C00_0000..=0x0DFF_FFFF` | Max 32 MB | Game Pak ROM (Waitstate 2) | Mirror Waitstate 2 |
| `0x0E00_0000..=0x0E00_FFFF` | 64 KB | Game Pak SRAM / Flash backup | Penyimpanan save data baterai |

#### Hardware Quirks yang Diimplementasikan:
1. **ARMv4T Unaligned 32-bit Read Rotation**:
   Pembacaan 32-bit (`LDR`) dari alamat yang tidak kelipatan 4 (`addr & 3 != 0`) tidak menyebabkan fault. Hardware membaca word 32-bit pada alamat `addr & !3`, kemudian melakukan perputaran bit ke kanan (*Rotate Right*) sebesar `(addr & 3) * 8` bit.
2. **Duplikasi Penulisan 8-bit ke VRAM & PALRAM**:
   Penulisan 8-bit ke PALRAM atau VRAM diduplikasi oleh hardware bus menjadi halfword 16-bit `val | (val << 8)` dan ditulis ke alamat halfword genap terdekat.
3. **Penulisan 8-bit ke OAM Diabaikan**:
   Hardware GBA mengabaikan penulisan byte tunggal ke OAM; hanya penulisan 16-bit atau 32-bit yang valid.

---

### 3. PPU & Graphics Engine

PPU GBA bekerja berdasarkan rasterisasi garis pindai (*scanline rasterizer*) dengan resolusi layar 240 x 160 piksel.

```mermaid
flowchart LR
    Scanline["Scanline Counter: 0 to 227"] --> Check{"Area Scanline"}
    Check -->|"Scanline 0 sampai 159"| Render["Render Scanline"]
    Check -->|"Scanline 160 sampai 227"| VBlank["VBlank Period"]

    subgraph COMP["Scanline Composition Pipeline"]
        BG["Render Backgrounds<br/>Mode 0: BG0-BG3 Text<br/>Mode 1: Text + Affine<br/>Mode 3-5: Direct Bitmap"]
        OBJ["Render Sprites (OAM)<br/>128 Objects (Index 127 down to 0)<br/>Normal & Affine Transformations"]
        Mixer["Priority & Alpha Mixer<br/>Layer Priority 0 sampai 3<br/>Sprite menang saat tie prioritas"]
        BG --> Mixer
        OBJ --> Mixer
    end

    Render --> BG
    Mixer --> FB["Frame Buffer (240x160 XRGB)"]
```

#### Karakteristik Grafis:
- **Timing Layar**:
  - 1 scanline = 1232 siklus CPU (960 siklus gambar HDRAW + 272 siklus HBLANK).
  - 1 frame = 228 scanlines (160 scanlines tampil + 68 scanlines VBLANK) = **280,896 siklus CPU per frame** (~59.73 Hz).
- **Mode Latar Belakang (BG Modes)**:
  - **Mode 0**: 4 Background teks independen (BG0–BG3), ukuran peta 32x32 hingga 64x64 tile, 4bpp (16 warna) atau 8bpp (256 warna), pembalikan horizontal/vertikal per tile, dan scrolling hardware per piksel via `BGnHOFS`/`BGnVOFS`.
  - **Mode 1**: Mode campuran (BG0 teks, BG1 teks, BG2 affine rotasi/skala).
  - **Mode 3**: Direct Color Bitmap 240x160 piksel, warna 16-bit BGR555 langsung dari VRAM tanpa palet.
  - **Mode 4**: 240x160 8-bit paletted bitmap dengan page-flipping ganda (Front/Back buffer).
  - **Mode 5**: 160x128 16-bit direct color bitmap dengan page-flipping ganda.
- **Hardware Sprites (OBJ)**:
  - 128 sprite independen dengan dimensi 8x8 hingga 64x64 (Square, Horizontal, Vertical).
  - Pembalikan horizontal/vertikal (H-Flip & V-Flip).
  - Pemetaan VRAM 1-Dimensi (1D tile sequential) dan 2-Dimensi (2D matrix 32-tile pitch).
  - Resolusi prioritas bertingkat (Priority 0–3) terhadap background layer.

---

### 4. DMA Controller

DMA Controller GBA menyediakan 4 channel transfer memori berkecepatan tinggi tanpa intervensi CPU.

```mermaid
flowchart TD
    Trigger["Start Timing Trigger"] -->|"Immediate (0)"| ImmRun["Transfer Instan saat Enable"]
    Trigger -->|"VBlank (1)"| VBRun["Transfer saat Scanline 160"]
    Trigger -->|"HBlank (2)"| HBRun["Transfer saat HBlank 0..159"]
    Trigger -->|"Special (3)"| SpRun["Direct Sound FIFO A/B"]

    subgraph EXEC_BLOCK["DMA Channel Execution (Prioritas: Ch0 ke Ch3)"]
        Load["Latch Internal SAD, DAD, & Word Count"]
        Loop["Loop Word / Halfword Transfers"]
        AddrCtrl["Update Alamat: Increment / Decrement / Fixed / Reload"]
        IRQ["Kirim IRQ Channel jika Diaktifkan"]
        Load --> Loop --> AddrCtrl --> IRQ
    end

    ImmRun --> Load
    VBRun --> Load
    HBRun --> Load
    SpRun --> Load
```

#### Spesifikasi Channel DMA:
- **Channel 0**: Prioritas tertinggi. Terbatas pada memori internal (tidak dapat mengakses ROM atau SRAM).
- **Channel 1 & 2**: Digunakan khusus untuk streaming audio Direct Sound FIFO A & B.
- **Channel 3**: Channel umum dengan kapasitas hitung word hingga 64K (65.536 transfer), mendukung transfer Cartridge ROM dan Game Pak DRQ.
- **Siklus Bus**: Menghabiskan 2 siklus inisialisasi + 2 siklus per transfer word/halfword, memajukan siklus sistem secara sinkron.

---

### 5. Programmable Timers

GBA memiliki 4 timer 16-bit independen (`Timer 0` hingga `Timer 3`).

| Register | Alamat | Fungsi |
| :--- | :--- | :--- |
| `TM0CNT_L` / `TM0CNT_H` | `0x0400_0100` / `0x0400_0102` | Counter & Kontrol Timer 0 |
| `TM1CNT_L` / `TM1CNT_H` | `0x0400_0104` / `0x0400_0106` | Counter & Kontrol Timer 1 |
| `TM2CNT_L` / `TM2CNT_H` | `0x0400_0108` / `0x0400_010A` | Counter & Kontrol Timer 2 |
| `TM3CNT_L` / `TM3CNT_H` | `0x0400_010C` / `0x0400_010E` | Counter & Kontrol Timer 3 |

- **Prescaler Clock**: Pilihan pembagi frekuensi sistem:
  - `00b`: Sistem Clock (16.78 MHz - 1 siklus per increment).
  - `01b`: Clock / 64 (64 siklus per increment).
  - `10b`: Clock / 256 (256 siklus per increment).
  - `11b`: Clock / 1024 (1024 siklus per increment).
- **Cascade Mode**: Bit 2 `Count-up Timing`. Saat aktif (hanya valid pada Timer 1–3), timer tidak berdetak dari clock prescaler, melainkan berdetak setiap kali timer sebelumnya mengalami overflow.
- **Overflow & Reload**: Saat nilai counter melampaui `0xFFFF`, counter secara otomatis memuat ulang (*reload*) nilai awal dari register `TMnCNT_L` dan memicu overflow interrupt jika bit 6 diaktifkan.

---

### 6. Audio Processing Unit (APU)

Sistem audio GBA menggabungkan sintesis PSG warisan Game Boy klasik dengan dua saluran streaming PCM Direct Sound modern.

```mermaid
flowchart LR
    subgraph PSG["Legacy PSG Channels"]
        Ch1["Channel 1<br/>Square Wave + Sweep + Envelope"]
        Ch2["Channel 2<br/>Square Wave + Envelope"]
        Ch3["Channel 3<br/>Wave Memory (32 Samples)"]
        Ch4["Channel 4<br/>White Noise LFSR"]
    end

    subgraph DSOUND["Direct Sound Channels"]
        FIFOA["Direct Sound A FIFO<br/>32-Byte Buffer"]
        FIFOB["Direct Sound B FIFO<br/>32-Byte Buffer"]
        TimerClock["Timer 0 / 1 Overflow"]
        TimerClock -->|"Pop Sample"| FIFOA
        TimerClock -->|"Pop Sample"| FIFOB
        FIFOA -.->|"Buffer Rendah"| DMA1["Picu DMA1 Burst"]
        FIFOB -.->|"Buffer Rendah"| DMA2["Picu DMA2 Burst"]
    end

    Ch1 --> Mixer["Stereo Audio Mixer & Limiter"]
    Ch2 --> Mixer
    Ch3 --> Mixer
    Ch4 --> Mixer
    FIFOA --> Mixer
    FIFOB --> Mixer
    Mixer --> Out["Audio Buffer Output<br/>44.1 kHz Stereo PCM"]
```

---

### 7. Interrupt Controller

Interrupt Controller menangani pengiriman exception hardware ke ARM7TDMI Core:
- **Register `IE` (`0x0400_0200`)**: Interrupt Enable bitmask.
- **Register `IF` (`0x0400_0202`)**: Interrupt Request flag. Penulisan nilai `1` pada bit tertentu akan membersihkan flag tersebut (*Acknowledge*).
- **Register `IME` (`0x0400_0208`)**: Interrupt Master Enable (1 = Seluruh interupsi diizinkan, 0 = Diblokir).
- **Prioritas Interupsi**: VBlank (bit 0), HBlank (bit 1), V-Counter Match (bit 2), Timer 0-3 (bits 3-6), Serial (bit 7), DMA 0-3 (bits 8-11), Keypad (bit 12), Game Pak (bit 13).

---

### 8. Keypad Controller

- **Register `KEYINPUT` (`0x0400_0130`)**: Status 10 tombol kontrol GBA (A, B, Select, Start, Right, Left, Up, Down, R, L). Bersifat **Active-Low** (nilai 0 = tombol sedang ditekan, 1 = dilepas).
- **Register `KEYCNT` (`0x0400_0132`)**: Keypad Interrupt Control untuk membangunkan CPU dari keadaan Stop/Halt.

---

## Siklus Sinkronisasi Emulasi (Cycle-Stepping)

Setiap panggilan ke `Gba::step()` memajukan CPU sebanyak satu instruksi, kemudian mendistribusikan jumlah siklus CPU tersebut ke seluruh periferal hardware:

```mermaid
sequenceDiagram
    autonumber
    participant App as Emulator Loop
    participant GBA as Coordinator
    participant CPU as ARM7TDMI
    participant Bus as GbaBus
    participant PPU as PPU Subsystem
    participant Timers as Timers
    participant APU as APU Audio

    App->>GBA: step()
    GBA->>CPU: step(bus)
    CPU->>Bus: Fetch, Decode, Execute
    Bus-->>CPU: Data / Opcode
    CPU-->>GBA: cycles executed

    GBA->>PPU: step(cycles)
    opt VBlank atau HBlank
        PPU-->>Bus: Request IRQ & Trigger DMA
    end

    GBA->>Bus: step_timers(cycles)
    opt Timer Overflow
        Bus->>APU: on_timer_overflow(id)
    end

    GBA->>APU: step(cycles)
    opt Pending IRQ dan IME Enabled
        GBA->>CPU: trigger_irq(bus)
    end
    GBA-->>App: total cycles
```

---

## Peta Register I/O GBA

| Alamat | Simbol | Ukuran | R/W | Deskripsi |
| :--- | :--- | :---: | :---: | :--- |
| `0x0400_0000` | `DISPCNT` | 16-bit | R/W | LCD Control: Mode Layar (0-5), Layer Enable (BG0-BG3, OBJ), 1D/2D mapping |
| `0x0400_0004` | `DISPSTAT` | 16-bit | R/W | General LCD Status: VBlank, HBlank, VCount Match flags & IRQ enable bits |
| `0x0400_0006` | `VCOUNT` | 16-bit | R | Scanline Counter saat ini (0..=227) |
| `0x0400_0008` | `BG0CNT` | 16-bit | R/W | Background 0 Control: Priority, Character Base, Screen Base, Size |
| `0x0400_000A` | `BG1CNT` | 16-bit | R/W | Background 1 Control |
| `0x0400_000C` | `BG2CNT` | 16-bit | R/W | Background 2 Control |
| `0x0400_000E` | `BG3CNT` | 16-bit | R/W | Background 3 Control |
| `0x0400_0010` | `BG0HOFS` | 16-bit | W | BG0 Horizontal Offset (Scroll X) |
| `0x0400_0012` | `BG0VOFS` | 16-bit | W | BG0 Vertical Offset (Scroll Y) |
| `0x0400_0060` | `SOUND1CNT_L`| 16-bit | R/W | Channel 1 Sweep register |
| `0x0400_0062` | `SOUND1CNT_H`| 16-bit | R/W | Channel 1 Duty cycle, Length, & Envelope |
| `0x0400_0064` | `SOUND1CNT_X`| 16-bit | R/W | Channel 1 Frequency & Initial trigger |
| `0x0400_0080` | `SOUNDCNT_L` | 16-bit | R/W | Stereo Sound Control (L/R volume per channel) |
| `0x0400_0082` | `SOUNDCNT_H` | 16-bit | R/W | Direct Sound Control & Output ratios |
| `0x0400_0084` | `SOUNDCNT_X` | 16-bit | R/W | Sound Master Enable & Activity status |
| `0x0400_0088` | `SOUNDBIAS`  | 16-bit | R/W | Sound Amplitude Bias (Default 0x0200) |
| `0x0400_00B0` | `DMA0SAD`    | 32-bit | W | DMA 0 Source Address |
| `0x0400_00B4` | `DMA0DAD`    | 32-bit | W | DMA 0 Destination Address |
| `0x0400_00B8` | `DMA0CNT_L`  | 16-bit | W | DMA 0 Word Count |
| `0x0400_00BA` | `DMA0CNT_H`  | 16-bit | R/W | DMA 0 Control (Enable, Timing, Repeat, IRQ) |
| `0x0400_0100` | `TM0CNT_L`   | 16-bit | R/W | Timer 0 Counter Value / Reload Value |
| `0x0400_0102` | `TM0CNT_H`   | 16-bit | R/W | Timer 0 Control (Prescaler, Cascade, IRQ Enable) |
| `0x0400_0130` | `KEYINPUT`   | 16-bit | R | Keypad Status (Active-Low) |
| `0x0400_0132` | `KEYCNT`     | 16-bit | R/W | Keypad Interrupt Control |
| `0x0400_0200` | `IE`         | 16-bit | R/W | Interrupt Enable Register |
| `0x0400_0202` | `IF`         | 16-bit | R/W | Interrupt Request / Acknowledge Register |
| `0x0400_0208` | `IME`        | 16-bit | R/W | Interrupt Master Enable Register |

---

## Kontrol Keyboard Desktop

| Tombol GBA | Tombol Keyboard | Fungsi pada Mode Demo |
| :--- | :--- | :--- |
| **D-Pad (Atas / Bawah / Kiri / Kanan)** | Tombol Panah Keyboard | Gerak Sprite Kursor & Hardware Scrolling (Mode 0) / Gerak Kursor (Mode 3) |
| **Tombol A** | `Z` | Ubah palet / aksi kursor |
| **Tombol B** | `X` | Aksi sekunder |
| **Start** | `Enter` | Tombol Start GBA |
| **Select** | `Backspace` atau `Spasi` | Tombol Select GBA |
| **Shoulder L / R** | `A` / `S` | Modulasi palet warna plasma (Mode 3) |
| **Toggle Mode** | `Tab` | Beralih instan antara **Mode 0 (Tilemap + Sprite)** dan **Mode 3 (Direct Bitmap)** |
| **Keluar** | `Escape` | Menutup emulator |

---

## Cara Kompilasi & Menjalankan

Pastikan toolchain Rust modern telah terpasang (`rustc 1.75+`).

### 1. Menjalankan Demo Interaktif Bawaan
Jika dijalankan tanpa parameter ROM, emulator secara otomatis menyajikan demo grafis interaktif Mode 0 (Tilemap Background dengan Hardware Sprite OAM) dan Mode 3 (Direct Color Plasma):

```bash
cargo run --release -p gba-desktop
```

### 2. Memuat dan Menjalankan ROM GBA
Untuk memuat ROM game (`.gba`):

```bash
cargo run --release -p gba-desktop -- path/to/game.gba
```

### 3. Kompilasi Release Teroptimasi
```bash
cargo build --release
```

---

## Pengujian & Verifikasi

Proyek ini dilengkapi dengan 49 pengujian unit dan integrasi otomatis yang memverifikasi seluruh subsistem hardware terhadap spesifikasi resmi GBATEK:

```bash
cargo test
```

### Cakupan Pengujian (49 Tests):
- **CPU**:
  - ALU arithmetic, carry not-borrow, dan signed overflow.
  - Barrel Shifter (LSL, LSR, ASR, ROR, RRX) dengan immediate dan register shifts.
  - CPSR & SPSR status decoding dan evaluasi kondisi branchless.
  - Register banking dan mode-switch swap (User/System, FIQ, IRQ, SVC).
  - ARM: Block transfer LDM/STM (IA, IB, DA, DB), Single Data LDR/STR, Branch Exchange (BX ARM <-> THUMB).
  - THUMB: Formats 1-19 (Shifted, Add/Sub, Immediate, Format 4 ALU, Push/Pop dengan PC reload).
- **Bus & Memory**:
  - Akses baca/tulis IWRAM, EWRAM, VRAM, PALRAM.
  - ARMv4T unaligned 32-bit read right-rotation.
  - Duplikasi penulisan byte 8-bit ke VRAM dan PALRAM.
- **DMA**:
  - Masking alamat sumber/tujuan 27-bit vs 28-bit.
  - Transfer seketika (*Immediate trigger*) antar-region EWRAM ke IWRAM.
  - Pemicuan sinkron VBlank dan HBlank.
  - Pipeline transfer Direct Sound Special Burst 16-byte ke FIFO A & B.
- **Timers**:
  - Prescaler /1, /64, /256, /1024 dan reload behavior.
  - Cascade mode (Timer 1 berdetak dari overflow Timer 0).
  - Pemicuan interupsi IRQ Timer.
- **PPU & Sprites**:
  - Konversi warna 15-bit BGR555 ke 32-bit ARGB/XRGB.
  - Rasterisasi Text Background Mode 0 dengan scrolling hardware (`BGnHOFS`).
  - Pembalikan tile horizontal (*H-Flip*) dan vertikal (*V-Flip*).
  - Rasterisasi Direct Bitmap Mode 3.
  - Hardware Sprite OAM (4bpp, 8bpp, H-Flip, V-Flip).
  - Resolusi prioritas hardware Sprite di atas Background.
- **APU**:
  - FIFO Push/Pop 32-bit.
  - Permintaan Special DMA saat buffer FIFO Direct Sound <= 16 byte.
  - Sintesis Channel 1 Square Wave dengan duty cycle.
- **Integrasi Penuh Sistem**:
  - Loop eksekusi CPU + PPU + Timer + Interrupt: Eksekusi program ARM menghasilkan VBlank IRQ yang mengalihkan CPU ke mode IRQ dan melompat ke vektor exception `0x0000_0018`.
  - Booting komersial Pokemon FireRed crt0 dan inisialisasi Tilemap Engine.

---

## Lisensi

Proyek ini dilisensikan di bawah pilihan lisensi:
- **MIT License** ([LICENSE-MIT](LICENSE-MIT) atau http://opensource.org/licenses/MIT)
- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE) atau http://www.apache.org/licenses/LICENSE-2.0)
