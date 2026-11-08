//! Original, tiny TrueType data for real kerning and registration fixtures.

use std::collections::BTreeMap;

pub(crate) const FAMILY: &str = "Local Caret Fixture";

fn u16_at(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn u32_at(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn rectangle(width: i16, height: i16) -> Vec<u8> {
    let mut glyph = vec![0; 34];
    u16_at(&mut glyph, 0, 1);
    u16_at(&mut glyph, 6, width as u16);
    u16_at(&mut glyph, 8, height as u16);
    u16_at(&mut glyph, 10, 3);
    glyph[14..18].fill(1);
    for (index, delta) in [0, width, 0, -width].into_iter().enumerate() {
        u16_at(&mut glyph, 18 + 2 * index, delta as u16);
    }
    for (index, delta) in [0, 0, height, 0].into_iter().enumerate() {
        u16_at(&mut glyph, 26 + 2 * index, delta as u16);
    }
    glyph
}

fn names() -> Vec<u8> {
    let names = [
        (1, FAMILY),
        (2, "Regular"),
        (4, "Local Caret Fixture Regular"),
        (6, "LocalCaretFixture-Regular"),
        (16, FAMILY),
    ];
    let start = 6 + names.len() * 12;
    let mut table = vec![0; start];
    u16_at(&mut table, 2, names.len() as u16);
    u16_at(&mut table, 4, start as u16);
    for (index, (id, value)) in names.into_iter().enumerate() {
        let encoded = value
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        let record = 6 + index * 12;
        for (offset, word) in [
            (0, 3),
            (2, 1),
            (4, 0x0409),
            (6, id),
            (8, encoded.len() as u16),
            (10, (table.len() - start) as u16),
        ] {
            u16_at(&mut table, record + offset, word);
        }
        table.extend_from_slice(&encoded);
    }
    table
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |sum, part| {
        let mut word = [0; 4];
        word[..part.len()].copy_from_slice(part);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

pub(crate) fn bytes() -> Vec<u8> {
    let mut tables = BTreeMap::new();
    let mut head = vec![0; 54];
    u32_at(&mut head, 0, 0x0001_0000);
    u32_at(&mut head, 4, 0x0001_0000);
    u32_at(&mut head, 12, 0x5f0f_3cf5);
    u16_at(&mut head, 18, 1000);
    u16_at(&mut head, 40, 600);
    u16_at(&mut head, 42, 700);
    u16_at(&mut head, 46, 8);
    u16_at(&mut head, 48, 2);
    tables.insert(*b"head", head);
    let mut hhea = vec![0; 36];
    u32_at(&mut hhea, 0, 0x0001_0000);
    u16_at(&mut hhea, 4, 800);
    u16_at(&mut hhea, 6, (-200i16) as u16);
    u16_at(&mut hhea, 10, 600);
    u16_at(&mut hhea, 18, 1);
    u16_at(&mut hhea, 34, 4);
    tables.insert(*b"hhea", hhea);
    let mut hmtx = vec![0; 16];
    for (glyph, advance) in [500, 500, 0, 600].into_iter().enumerate() {
        u16_at(&mut hmtx, glyph * 4, advance);
    }
    tables.insert(*b"hmtx", hmtx);
    let mut maxp = vec![0; 32];
    u32_at(&mut maxp, 0, 0x0001_0000);
    u16_at(&mut maxp, 4, 4);
    u16_at(&mut maxp, 6, 4);
    u16_at(&mut maxp, 8, 1);
    u16_at(&mut maxp, 14, 1);
    tables.insert(*b"maxp", maxp);
    let mut cmap = vec![0; 60];
    u16_at(&mut cmap, 2, 1);
    u16_at(&mut cmap, 4, 3);
    u16_at(&mut cmap, 6, 1);
    u32_at(&mut cmap, 8, 12);
    for (offset, value) in [(12, 4), (14, 48), (18, 8), (20, 8), (22, 2)] {
        u16_at(&mut cmap, offset, value);
    }
    for (index, code) in [0x0041u16, 0x0042, 0x0301, 0xffff].into_iter().enumerate() {
        u16_at(&mut cmap, 26 + index * 2, code);
        u16_at(&mut cmap, 36 + index * 2, code);
        let glyph = [1u16, 3, 2, 0][index];
        u16_at(&mut cmap, 44 + index * 2, glyph.wrapping_sub(code));
    }
    tables.insert(*b"cmap", cmap);
    let mut kern = vec![0; 36];
    u16_at(&mut kern, 2, 1);
    u16_at(&mut kern, 6, 32);
    u16_at(&mut kern, 8, 1);
    u16_at(&mut kern, 10, 3);
    u16_at(&mut kern, 12, 12);
    u16_at(&mut kern, 14, 1);
    u16_at(&mut kern, 16, 6);
    for (index, (left, right, value)) in [(1, 2, 5i16), (2, 2, 10), (2, 3, -25)]
        .into_iter()
        .enumerate()
    {
        let offset = 18 + index * 6;
        u16_at(&mut kern, offset, left);
        u16_at(&mut kern, offset + 2, right);
        u16_at(&mut kern, offset + 4, value as u16);
    }
    tables.insert(*b"kern", kern);
    let mut glyf = vec![0; 10];
    glyf.extend(rectangle(400, 700));
    glyf.extend(rectangle(300, 200));
    glyf.extend(rectangle(550, 700));
    tables.insert(*b"glyf", glyf);
    let mut loca = vec![0; 10];
    for (index, offset) in [0, 5, 22, 39, 56].into_iter().enumerate() {
        u16_at(&mut loca, index * 2, offset);
    }
    tables.insert(*b"loca", loca);
    tables.insert(*b"name", names());
    let mut os2 = vec![0; 78];
    u16_at(&mut os2, 2, 500);
    u16_at(&mut os2, 4, 400);
    u16_at(&mut os2, 6, 5);
    os2[58..62].copy_from_slice(b"TEST");
    u16_at(&mut os2, 62, 0x0040);
    u16_at(&mut os2, 64, 0x0041);
    u16_at(&mut os2, 66, 0x0301);
    u16_at(&mut os2, 68, 800);
    u16_at(&mut os2, 70, (-200i16) as u16);
    u16_at(&mut os2, 74, 800);
    u16_at(&mut os2, 76, 200);
    tables.insert(*b"OS/2", os2);
    let mut post = vec![0; 32];
    u32_at(&mut post, 0, 0x0003_0000);
    u16_at(&mut post, 8, (-75i16) as u16);
    u16_at(&mut post, 10, 50);
    tables.insert(*b"post", post);
    let count = tables.len();
    let mut font = vec![0; 12 + count * 16];
    u32_at(&mut font, 0, 0x0001_0000);
    u16_at(&mut font, 4, count as u16);
    u16_at(&mut font, 6, 128);
    u16_at(&mut font, 8, 3);
    u16_at(&mut font, 10, (count * 16 - 128) as u16);
    let mut head_offset = 0;
    for (index, (tag, data)) in tables.into_iter().enumerate() {
        while font.len() % 4 != 0 {
            font.push(0);
        }
        let offset = font.len();
        let record = 12 + index * 16;
        font[record..record + 4].copy_from_slice(&tag);
        u32_at(&mut font, record + 4, checksum(&data));
        u32_at(&mut font, record + 8, offset as u32);
        u32_at(&mut font, record + 12, data.len() as u32);
        if tag == *b"head" {
            head_offset = offset;
        }
        font.extend(data);
    }
    while font.len() % 4 != 0 {
        font.push(0);
    }
    let adjustment = 0xb1b0_afbau32.wrapping_sub(checksum(&font));
    u32_at(&mut font, head_offset + 8, adjustment);
    font
}
