use pantry::math::Vec3;
use pantry::serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Position on the chunk grid. One unit on `x` or `z` is `CHUNK_SIZE_METERS`; `y` is global.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkCoord {
    pub x: i32,
    pub z: i32,
}

impl ChunkCoord {
    pub const CHUNK_SIZE_METERS: f32 = 128.0;

    pub fn new(x: i32, z: i32) -> Self {
        ChunkCoord { x, z }
    }

    /// World translation from the world origin to this chunk's local origin.
    #[allow(clippy::cast_precision_loss)]
    pub fn to_world_offset(self) -> Vec3 {
        Vec3::new(
            self.x as f32 * Self::CHUNK_SIZE_METERS,
            0.0,
            self.z as f32 * Self::CHUNK_SIZE_METERS,
        )
    }
}

impl Serialize for ChunkCoord {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        [self.x, self.z].serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ChunkCoord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let [x, z] = <[i32; 2]>::deserialize(deserializer)?;
        Ok(ChunkCoord { x, z })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_offset_at_origin_is_zero() {
        assert_eq!(ChunkCoord::new(0, 0).to_world_offset(), Vec3::ZERO);
    }

    #[test]
    fn world_offset_scales_by_chunk_size() {
        let c = ChunkCoord::new(2, -1);
        assert_eq!(c.to_world_offset(), Vec3::new(256.0, 0.0, -128.0));
    }

    #[test]
    fn serializes_as_two_element_array() {
        let c = ChunkCoord::new(3, -4);
        let json = pantry::serde_json::to_string(&c).unwrap();
        assert_eq!(json, "[3,-4]");
        let back: ChunkCoord = pantry::serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }
}
