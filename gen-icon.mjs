// Minimal ICO generator: single 32x32 RGBA blue square
import { writeFileSync } from 'fs'

function makeIco(size) {
  const pixels = size * size * 4
  const bmpHeaderSize = 40
  const totalSize = bmpHeaderSize + pixels * 2 // XOR + AND mask

  const ico = Buffer.alloc(6 + 16 + bmpHeaderSize + pixels + (size * 4))

  // ICO header
  ico.writeUInt16LE(0, 0)       // reserved
  ico.writeUInt16LE(1, 2)       // type: 1=ICO
  ico.writeUInt16LE(1, 4)       // count: 1 image

  // Directory entry
  const dataOffset = 6 + 16
  const dataSize = bmpHeaderSize + pixels + (size * size / 8)
  ico.writeUInt8(size, 6)
  ico.writeUInt8(size, 7)
  ico.writeUInt8(0, 8)          // color count
  ico.writeUInt8(0, 9)          // reserved
  ico.writeUInt16LE(1, 10)      // color planes
  ico.writeUInt16LE(32, 12)     // bits per pixel
  ico.writeUInt32LE(dataSize, 14)
  ico.writeUInt32LE(dataOffset, 18)

  // BITMAPINFOHEADER
  let o = dataOffset
  ico.writeUInt32LE(40, o); o += 4
  ico.writeInt32LE(size, o); o += 4
  ico.writeInt32LE(size * 2, o); o += 4   // height * 2 for ICO
  ico.writeUInt16LE(1, o); o += 2         // planes
  ico.writeUInt16LE(32, o); o += 2        // bit count
  ico.writeUInt32LE(0, o); o += 4         // compression
  ico.writeUInt32LE(pixels, o); o += 4
  ico.writeInt32LE(0, o); o += 4
  ico.writeInt32LE(0, o); o += 4
  ico.writeUInt32LE(0, o); o += 4
  ico.writeUInt32LE(0, o); o += 4

  // Pixel data (BGRA, bottom-up)
  for (let y = size - 1; y >= 0; y--) {
    for (let x = 0; x < size; x++) {
      ico.writeUInt8(0x60, o++)   // B
      ico.writeUInt8(0x90, o++)   // G
      ico.writeUInt8(0xFF, o++)   // R
      ico.writeUInt8(0xFF, o++)   // A
    }
  }

  // AND mask (all transparent = 0)
  const maskBytes = size * size / 8
  for (let i = 0; i < maskBytes; i++) ico.writeUInt8(0, o++)

  return ico
}

writeFileSync('src-tauri/icons/icon.ico', makeIco(32))
console.log('icon.ico created')
