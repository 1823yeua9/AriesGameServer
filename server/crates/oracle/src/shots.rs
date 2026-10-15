//! Shot-time transforms from the reusable hitscan projectile, not the camera
//! after recoil. Layout: 13.06 SDK UFiringStateComponent and AProjectile.

use crate::client::LiveClient;
use crate::world::Camera;
use crate::{Memory, OracleError};

pub struct ShotSample {
    pub camera: Camera,
    pub identity: (u64, u32, u32),
}

pub fn read_shot(
    memory: &impl Memory,
    firing: u64,
    rng: u64,
    expected_seed: u32,
) -> Result<ShotSample, OracleError> {
    if memory.u32(rng + 0xec)? != expected_seed {
        return Err(OracleError::Invalid("Firing seed changed before shot read"));
    }

    let projectile = memory.ptr(firing + 0x4f8)?;
    let raw: [u8; 0xb0] = memory.read(projectile + 0x500)?;
    let time_bits = u32::from_le_bytes(raw[0x18..0x1c].try_into().unwrap());
    let creation_seed = u32::from_le_bytes(raw[0x48..0x4c].try_into().unwrap());
    let time = f32::from_bits(time_bits);
    let value = |offset| f64::from_le_bytes(raw[offset..offset + 8].try_into().unwrap());
    let rotation: [f64; 4] = std::array::from_fn(|index| value(0x50 + index * 8));
    let location: [f64; 3] = std::array::from_fn(|index| value(0x70 + index * 8));

    if !time.is_finite()
        || time <= 0.0
        || location
            .iter()
            .chain(rotation.iter())
            .any(|v| !v.is_finite())
        || (rotation.iter().map(|v| v * v).sum::<f64>() - 1.0).abs() > 0.001
    {
        return Err(OracleError::Invalid(
            "Invalid projectile creation transform",
        ));
    }

    // Check both the shot identity and producer seed after the bounded read.
    if memory.u64(firing + 0x4f8)? != projectile
        || memory.u32(projectile + 0x518)? != time_bits
        || memory.u32(projectile + 0x548)? != creation_seed
        || memory.u32(rng + 0xec)? != expected_seed
    {
        return Err(OracleError::Invalid("Projectile changed during shot read"));
    }

    let [x, y, z, w] = rotation;
    let direction = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + w * z),
        2.0 * (x * z - w * y),
    ];
    let pitch = direction[2]
        .atan2(direction[0].hypot(direction[1]))
        .to_degrees();
    let yaw = direction[1].atan2(direction[0]).to_degrees();

    Ok(ShotSample {
        camera: Camera {
            location,
            rotation: [pitch, yaw, 0.0],
        },
        identity: (projectile, time_bits, creation_seed),
    })
}

#[cfg(test)]
mod tests {
    use super::read_shot;
    use crate::{FakeMemory, Memory, OracleError};
    use std::cell::Cell;

    fn fixture() -> FakeMemory {
        let mut memory = FakeMemory::new(0x10000);
        memory.write_u64(0x204f8, 0x30000);
        memory.write(0x30500, &[0; 0xb0]);
        memory.write_u32(0x30518, 12.5_f32.to_bits());
        memory.write_u32(0x30548, 123);
        memory.write_u32(0x400ec, 456);
        memory.write_f64(0x30550 + 24, 1.0);
        for (index, value) in [100.0, 200.0, 300.0].into_iter().enumerate() {
            memory.write_f64(0x30570 + index as u64 * 8, value);
        }
        memory
    }

    #[test]
    fn reads_shot_pose_without_render_camera_and_keeps_spread() {
        let mut memory = fixture();
        let half_pitch = 5.0_f64.to_radians() / 2.0;
        memory.write_f64(0x30558, -half_pitch.sin());
        memory.write_f64(0x30568, half_pitch.cos());
        let shot = read_shot(&memory, 0x20000, 0x40000, 456).unwrap();
        assert_eq!(shot.camera.location, [100.0, 200.0, 300.0]);
        assert!((shot.camera.rotation[0] - 5.0).abs() < 1e-9);
        assert_eq!(shot.identity, (0x30000, 12.5_f32.to_bits(), 123));
    }

    #[test]
    fn rejects_wrong_seed_uninitialized_and_invalid_transforms() {
        let mut memory = fixture();
        assert!(read_shot(&memory, 0x20000, 0x40000, 999).is_err());
        memory.write_u32(0x30518, 0);
        assert!(read_shot(&memory, 0x20000, 0x40000, 456).is_err());
        memory.write_u32(0x30518, 12.5_f32.to_bits());
        memory.write_f64(0x30568, 2.0);
        assert!(read_shot(&memory, 0x20000, 0x40000, 456).is_err());
    }

    #[test]
    fn rejects_a_seed_change_during_the_transform_read() {
        struct UpdatingMemory {
            memory: FakeMemory,
            changed: Cell<bool>,
        }

        impl Memory for UpdatingMemory {
            fn image_base(&self) -> u64 {
                self.memory.image_base()
            }

            fn read_into(&self, address: u64, buffer: &mut [u8]) -> Result<(), OracleError> {
                self.memory.read_into(address, buffer)?;

                if address == 0x30500 {
                    self.changed.set(true);
                }
                if address == 0x400ec && self.changed.get() {
                    buffer.copy_from_slice(&789_u32.to_le_bytes());
                }
                Ok(())
            }
        }

        let memory = UpdatingMemory {
            memory: fixture(),
            changed: Cell::new(false),
        };
        assert!(read_shot(&memory, 0x20000, 0x40000, 456).is_err());
    }
}

impl<M: Memory> LiveClient<M> {
    pub fn firing_camera(
        &mut self,
        state_machine: u64,
        key: &str,
        seed: u32,
    ) -> Result<Option<Camera>, OracleError> {
        let cache_key = (state_machine, key.to_string());
        let states = self.state_names(state_machine)?;
        let firing = states
            .iter()
            .find(|(name, _)| name == "FiringState")
            .or_else(|| states.iter().find(|(name, _)| name.starts_with("Firing")))
            .map(|(_, address)| *address)
            .ok_or(OracleError::Invalid("Firing state unavailable"))?;
        let rng = self.memory.ptr(firing + 0x4a0)?;
        let shot = read_shot(&self.memory, firing, rng, seed)?;

        if self.caches.last_shots.get(&cache_key) == Some(&shot.identity) {
            return Ok(None);
        }

        self.caches.last_shots.insert(cache_key, shot.identity);
        Ok(Some(shot.camera))
    }
}
