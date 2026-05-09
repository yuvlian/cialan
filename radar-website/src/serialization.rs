use radar_dumper::Overview;
use radar_reader::Player;

pub struct BinaryWriter {
    buffer: Vec<u8>,
}

impl BinaryWriter {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(cap),
        }
    }

    pub fn write_u8(&mut self, val: u8) {
        self.buffer.push(val);
    }

    pub fn write_i16(&mut self, val: i16) {
        self.buffer.extend_from_slice(&val.to_le_bytes());
    }

    pub fn write_u16(&mut self, val: u16) {
        self.buffer.extend_from_slice(&val.to_le_bytes());
    }

    pub fn write_string(&mut self, val: &str) {
        let bytes = val.as_bytes();
        let len = bytes.len() as u8;
        self.write_u8(len);
        self.buffer.extend_from_slice(&bytes[..len as usize]);
    }

    pub fn finish(self) -> Vec<u8> {
        self.buffer
    }
}

#[derive(Clone)]
pub struct WebState {
    pub map_name: String,
    pub overview: Option<Overview>,
    pub players: Vec<Player>,
}

impl WebState {
    pub fn size_hint(&self) -> usize {
        let mut size = 1 + self.map_name.len();
        size += 1; // has_overview
        if let Some(ov) = &self.overview {
            size += 4; // pos_x, pos_y (2 * i16)
            size += 2; // scale (u16 fixed-point)
            size += 1; // vertical sections count
            for vs in &ov.vertical_sections {
                size += 1 + vs.name.len() + 4; // name + 2 * i16
            }
        }
        size += 1; // players count
        for p in &self.players {
            size += 1 + p.name.len() + 1 + 1 + 6; // name + health + team + pos (3 * i16)
        }
        size
    }

    pub fn to_binary(&self) -> Vec<u8> {
        let mut w = BinaryWriter::with_capacity(self.size_hint());
        w.write_string(&self.map_name);

        if let Some(ov) = &self.overview {
            w.write_u8(1);
            w.write_i16(ov.pos_x);
            w.write_i16(ov.pos_y);
            w.write_u16((ov.scale * 1000.0) as u16);
            w.write_u8(ov.vertical_sections.len() as u8);
            for vs in &ov.vertical_sections {
                w.write_string(&vs.name);
                w.write_i16(vs.altitude_max);
                w.write_i16(vs.altitude_min);
            }
        } else {
            w.write_u8(0);
        }

        w.write_u8(self.players.len() as u8);
        for p in &self.players {
            w.write_string(&p.name);
            w.write_u8(p.health as u8);
            w.write_u8(p.team);
            w.write_i16(p.pos[0] as i16);
            w.write_i16(p.pos[1] as i16);
            w.write_i16(p.pos[2] as i16);
        }
        w.finish()
    }
}
