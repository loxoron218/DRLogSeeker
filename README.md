# DRLogSeeker

[![License: GPLv3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![Built with Vibe](https://img.shields.io/badge/Built%20With-▚Cursor%20AI%20Vibe-7B42F6.svg)](https://cursor.so)

**GTK4/Libadwaita/Rust tool for analyzing DR logs from MAAT DROffline and foobar2000**  
*"Because sometimes you don't have time to wait for a torrent tracker invite"*

![DRLogSeeker Interface](https://github.com/user-attachments/assets/70de874c-69e1-494d-9c25-2191323f1053)

## Features
- **Dual-Language DR Detection** - Finds both English/Russian DR log formats
- **Color-Coded for Your Sanity** - Full-spectrum color mapping of DR values (0-14)
- **Parallelized Analysis** - Rayon-powered scanning for massive log collections
- **Filesystem Integration** - Optional destructive operations for clean workflows
- **Keyboard Controls** - Keyboard-driven interface with vim-like controls

## Installation

### Arch Linux (AUR) (Coming Soon)
```bash
yay -S drlogseeker
```

### Flatpak (Coming Soon)
```bash
flatpak install flathub com.github.drlogseeker
```

## DR Chromesthesia System

| DR Range | Color       | Preview   | Audio Health         |
|----------|-------------|-----------|----------------------|
| 0-7      | Red      | <span style="color: #FF0000">██</span> | Very Compressed      |
| 8        | Burnt    | <span style="color: #FF4800">██</span> | Highly Compressed    |
| 9        | Orange   | <span style="color: #FF9100">██</span> | Compressed           |
| 10       | Yellow   | <span style="color: #FFD900">██</span> | Slightly Compressed  |
| 11       | Lime     | <span style="color: #D9FF00">██</span> | Moderate             |
| 12       | Mint     | <span style="color: #90FF00">██</span> | Good                 |
| 13       | Green    | <span style="color: #48FF00">██</span> | Very Good            |
| 14       | Neon     | <span style="color: #0F0">██</span>    | Excellent            |

## Basic Usage
1. **Search Logs**  
   Type in your favorite album on Nicotine+
2. **Filter Text Files**  
   Press `Ctrl+F` to search and choose 'Text' from the filetype menu. Then press`Ctrl+A` to select all files and download them
3. **Batch Analyze**  
   Open DRLogSeeker and pick the folder all the files *.txt/log are located in and scan

## Development

```bash
git clone https://github.com/loxoron218/DRLogSeeker
cd DRLogSeeker
cargo build --release
```

## Contributing
All contributions are welcomed, specially:
- GTK4 CSS wizards
- Rust parallelism shamans
- i18n/Russian language specialists
- AUR/Fedora/Flatpak packaging experts

## Related Projects
- [DR Loudness War](https://dr.loudness-war.info/) - DR database
- [Nicotine+](https://nicotine-plus.org/) - Open source Soulseek client
- [Soulseek](https://www.slsknet.org/) - Official P2P file-sharing network
