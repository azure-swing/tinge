pub fn dng() -> Vec<u8> {
    dng_with(Vec::new(), None)
}
pub fn dng_with(overrides: Vec<(u16, u16, u32, Vec<u8>)>, samples: Option<Vec<u16>>) -> Vec<u8> {
    let short = |x: u16| x.to_le_bytes().to_vec();
    let long = |x: u32| x.to_le_bytes().to_vec();
    let rational = |x: i32| [x.to_le_bytes(), 1_000_000i32.to_le_bytes()].concat();
    let mut tags: Vec<(u16, u16, u32, Vec<u8>)> = vec![
        (254, 4, 1, long(0)),
        (256, 4, 1, long(32)),
        (257, 4, 1, long(32)),
        (258, 3, 1, short(16)),
        (259, 3, 1, short(1)),
        (262, 3, 1, short(32803)),
        (271, 2, 10, b"VibeColor\0".to_vec()),
        (272, 2, 13, b"Test sensor\0\0".to_vec()),
        (273, 4, 1, long(0)),
        (274, 3, 1, short(1)),
        (277, 3, 1, short(1)),
        (278, 4, 1, long(32)),
        (279, 4, 1, long(32 * 32 * 2)),
        (284, 3, 1, short(1)),
        (33421, 3, 2, [short(2), short(2)].concat()),
        (33422, 1, 4, vec![0, 1, 1, 2]),
        (50706, 1, 4, vec![1, 4, 0, 0]),
        (50707, 1, 4, vec![1, 1, 0, 0]),
        (50708, 2, 13, b"Test sensor\0\0".to_vec()),
        (50710, 1, 3, vec![0, 1, 2]),
        (50711, 3, 1, short(1)),
        (
            50714,
            5,
            1,
            [64u32.to_le_bytes(), 1u32.to_le_bytes()].concat(),
        ),
        (50717, 4, 1, long(4095)),
        (
            50721,
            10,
            9,
            [
                3_240_454, -1_537_139, -498_531, -969_266, 1_876_011, 41_556, 55_643, -204_026,
                1_057_225,
            ]
            .into_iter()
            .flat_map(rational)
            .collect(),
        ),
        (
            50728,
            5,
            3,
            [1u32.to_le_bytes(), 1u32.to_le_bytes()].concat().repeat(3),
        ),
        (50778, 3, 1, short(21)),
    ];
    for t in overrides {
        tags.retain(|old| old.0 != t.0);
        tags.push(t);
    }
    tags.sort_by_key(|t| t.0);
    let header_size = 8 + 2 + tags.len() * 12 + 4;
    let mut extra = Vec::new();
    let mut entries = Vec::new();
    for (tag, typ, count, data) in tags {
        entries.extend(tag.to_le_bytes());
        entries.extend(typ.to_le_bytes());
        entries.extend(count.to_le_bytes());
        if data.len() <= 4 {
            let mut value = [0; 4];
            value[..data.len()].copy_from_slice(&data);
            entries.extend(value);
        } else {
            entries.extend(((header_size + extra.len()) as u32).to_le_bytes());
            extra.extend(data);
            if extra.len() % 2 == 1 {
                extra.push(0);
            }
        }
    }
    let strip = (header_size + extra.len()) as u32;
    for entry in entries.chunks_exact_mut(12) {
        if u16::from_le_bytes(entry[..2].try_into().unwrap()) == 273 {
            entry[8..12].copy_from_slice(&strip.to_le_bytes());
        }
    }
    let mut result = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
    result.extend(((entries.len() / 12) as u16).to_le_bytes());
    result.extend(entries);
    result.extend([0; 4]);
    result.extend(extra);
    let mut values = samples.unwrap_or_default().into_iter();
    for y in 0..32 {
        for x in 0..32 {
            let value: u16 = values.next().unwrap_or(match (x % 2, y % 2) {
                (0, 0) => 1664,
                (1, 1) => 864,
                _ => 1264,
            });
            result.extend(value.to_le_bytes());
        }
    }
    result
}
