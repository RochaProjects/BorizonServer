//! Dynamic vanilla registries sent during Minecraft 26.3 configuration.
//!
//! This list is pinned to `RegistryDataLoader.SYNCHRONIZED_REGISTRIES` from
//! the official 26.3 **server** jar. It is protocol/bootstrap metadata, not a
//! client registry or client resource list. The entries are loaded from a
//! compact server-data bundle produced by the future server-only preparer.

use horizon_protocol::packets::{
    Empty,
    configuration::{self, RegistryData, RegistryEntry, RegistryTags, TagEntry, UpdateTags},
};
use horizon_world::nbt::{self, Compound, JavaString, Limits, Tag};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Registry identifiers the 26.3 server synchronizes before entering play.
/// Keep Mojang's declaration order: it is the reference for the configuration
/// packet sequence and makes probes deterministic.
pub const SYNCHRONIZED_REGISTRIES: &[&str] = &[
    "minecraft:worldgen/biome",
    "minecraft:chat_type",
    "minecraft:trim_pattern",
    "minecraft:trim_material",
    "minecraft:wolf_variant",
    "minecraft:wolf_sound_variant",
    "minecraft:pig_variant",
    "minecraft:pig_sound_variant",
    "minecraft:frog_variant",
    "minecraft:cat_variant",
    "minecraft:cat_sound_variant",
    "minecraft:cow_sound_variant",
    "minecraft:cow_variant",
    "minecraft:chicken_sound_variant",
    "minecraft:chicken_variant",
    "minecraft:zombie_nautilus_variant",
    "minecraft:painting_variant",
    "minecraft:sulfur_cube_archetype",
    "minecraft:dimension_type",
    "minecraft:damage_type",
    "minecraft:banner_pattern",
    "minecraft:enchantment",
    "minecraft:jukebox_song",
    "minecraft:instrument",
    "minecraft:test_environment",
    "minecraft:test_instance",
    "minecraft:dialog",
    "minecraft:world_clock",
    "minecraft:timeline",
    "minecraft:decorated_pot_pattern",
    "minecraft:block_transformer",
    "minecraft:worldgen/block_state_provider",
];

/// All vanilla registry payloads needed to finish configuration for 26.3.
#[derive(Clone, Debug)]
pub struct RegistryBundle {
    registries: Vec<RegistryData>,
    tags: Vec<RegistryTags>,
}

impl RegistryBundle {
    #[cfg(test)]
    pub(crate) fn from_test_parts(registries: Vec<RegistryData>, tags: Vec<RegistryTags>) -> Self {
        Self { registries, tags }
    }

    /// Load prepared NBT values from `registry-sync/<namespace>/<registry path>.nbt`.
    ///
    /// Each NBT file is a root compound keyed by resource ID. Values wrap the
    /// optional vanilla NBT codec representation so entries known from a pack
    /// remain present even when their packet data is omitted. Java's server-side
    /// preparer writes these using the pinned server jar's codecs.
    pub fn load(resources: &Path) -> Result<Self, String> {
        // NBT compounds are maps and do not preserve registry ID order. Vanilla
        // assigns the dynamic numeric IDs from RegistrySynchronization's list
        // order, so the server-data exporter writes this separate ordered index.
        let order_path = resources.join("registry-order.json");
        let order_bytes = fs::read(&order_path).map_err(|error| {
            format!(
                "read synchronized registry order ({}): {error}",
                order_path.display()
            )
        })?;
        let order: serde_json::Value = serde_json::from_slice(&order_bytes)
            .map_err(|error| format!("parse synchronized registry order: {error}"))?;
        let order = order
            .as_object()
            .ok_or("registry-order.json root must be an object")?;
        if order.len() != SYNCHRONIZED_REGISTRIES.len()
            || SYNCHRONIZED_REGISTRIES
                .iter()
                .any(|id| !order.contains_key(*id))
        {
            return Err(
                "registry-order.json does not list exactly the pinned synchronized registries"
                    .into(),
            );
        }
        let mut registries = Vec::with_capacity(SYNCHRONIZED_REGISTRIES.len());
        for id in SYNCHRONIZED_REGISTRIES {
            let path = registry_file(resources, id);
            let bytes = fs::read(&path).map_err(|error| {
                format!(
                    "read synchronized registry {id} ({}): {error}",
                    path.display()
                )
            })?;
            let (root, used) = nbt::read_named(&bytes, Limits::default()).map_err(|error| {
                format!(
                    "decode synchronized registry {id} ({}): {error}",
                    path.display()
                )
            })?;
            if used != bytes.len() {
                return Err(format!(
                    "synchronized registry {id} has {} trailing NBT bytes",
                    bytes.len() - used
                ));
            }
            let Tag::Compound(entries) = root.tag else {
                return Err(format!("synchronized registry {id} root is not a compound"));
            };
            if entries.is_empty() {
                return Err(format!("synchronized registry {id} has no entries"));
            }
            let ordered_ids = order
                .get(*id)
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| format!("registry-order.json has no ordered ID list for {id}"))?;
            if ordered_ids.len() != entries.len() {
                return Err(format!(
                    "registry {id} order has {} IDs but its NBT contains {} entries",
                    ordered_ids.len(),
                    entries.len()
                ));
            }
            let entries = decode_ordered_entries(id, ordered_ids, entries)?;
            registries.push(RegistryData {
                registry: (*id).to_owned(),
                entries,
            });
        }
        let tags = load_tags(resources)?;
        Ok(Self { registries, tags })
    }

    pub fn registries(&self) -> &[RegistryData] {
        &self.registries
    }

    /// Dynamic registry ID assigned by the ordered CONFIGURATION packet.
    pub fn entry_index(&self, registry: &str, id: &str) -> Option<i32> {
        let entries = &self
            .registries
            .iter()
            .find(|candidate| candidate.registry == registry)?
            .entries;
        entries
            .iter()
            .position(|entry| entry.id == id)
            .and_then(|index| i32::try_from(index).ok())
    }

    pub fn entry_count(&self) -> usize {
        self.registries
            .iter()
            .map(|registry| registry.entries.len())
            .sum()
    }

    pub fn tag_count(&self) -> usize {
        self.tags.iter().map(|registry| registry.tags.len()).sum()
    }

    /// Encode the ordered `ClientboundRegistryDataPacket`s for CONFIGURATION.
    pub fn encode_packets(&self) -> impl Iterator<Item = Vec<u8>> + '_ {
        self.registries
            .iter()
            .cloned()
            .map(|registry| configuration::Clientbound::RegistryData(registry).encode())
    }

    /// Build the CONFIGURATION packets in vanilla order. A connection handler
    /// must send the first two packets, read the client's known-pack response,
    /// then send the remaining packets. The default feature and known-pack
    /// lists are empty; tags come from `registry-tags.nbt`.
    pub fn encode_configuration_packets(&self) -> Vec<Vec<u8>> {
        let mut packets = Vec::with_capacity(self.registries.len() + 4);
        packets.push(
            configuration::Clientbound::UpdateEnabledFeatures(
                configuration::UpdateEnabledFeatures {
                    features: Vec::new(),
                },
            )
            .encode(),
        );
        packets.push(
            configuration::Clientbound::SelectKnownPacks(configuration::SelectKnownPacks {
                packs: Vec::new(),
            })
            .encode(),
        );
        packets.extend(self.encode_packets());
        packets.push(
            configuration::Clientbound::UpdateTags(UpdateTags {
                registries: self.tags.clone(),
            })
            .encode(),
        );
        packets.push(configuration::Clientbound::FinishConfiguration(Empty).encode());
        packets
    }
}

fn decode_ordered_entries(
    registry: &str,
    ordered_ids: &[serde_json::Value],
    mut encoded_entries: Compound,
) -> Result<Vec<RegistryEntry>, String> {
    let mut seen = std::collections::HashSet::with_capacity(ordered_ids.len());
    let decoded = ordered_ids
        .iter()
        .map(|value| {
            let name = value
                .as_str()
                .ok_or_else(|| format!("registry {registry} order contains a non-string ID"))?;
            if !valid_resource_id(name) {
                return Err(format!(
                    "registry {registry} contains invalid entry id {name:?}"
                ));
            }
            if !seen.insert(name) {
                return Err(format!(
                    "registry {registry} order repeats entry id {name:?}"
                ));
            }
            let encoded = encoded_entries
                .remove(&JavaString::from(name))
                .ok_or_else(|| {
                    format!("registry {registry} order references missing entry {name}")
                })?;
            let Tag::Compound(wrapper) = encoded else {
                return Err(format!(
                    "registry {registry} entry {name} is not an encoded compound"
                ));
            };
            let wrapper = Tag::Compound(wrapper);
            let data = match wrapper.get("_borizon_data_present") {
                Some(Tag::Byte(0)) if wrapper.get("data").is_none() => None,
                Some(Tag::Byte(1)) => Some(
                    wrapper
                        .get("data")
                        .filter(|data| **data != Tag::End)
                        .cloned()
                        .ok_or_else(|| {
                            format!("registry {registry} entry {name} is missing its NBT data")
                        })?,
                ),
                _ => {
                    return Err(format!(
                        "registry {registry} entry {name} has an invalid optional-data marker"
                    ));
                }
            };
            Ok(RegistryEntry {
                id: name.to_owned(),
                data,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !encoded_entries.is_empty() {
        return Err(format!(
            "registry {registry} NBT contains IDs absent from registry-order.json"
        ));
    }
    Ok(decoded)
}

fn load_tags(resources: &Path) -> Result<Vec<RegistryTags>, String> {
    let path = resources.join("registry-tags.nbt");
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "read synchronized registry tags ({}): {error}",
            path.display()
        )
    })?;
    let (root, used) = nbt::read_named(&bytes, Limits::default()).map_err(|error| {
        format!(
            "decode synchronized registry tags ({}): {error}",
            path.display()
        )
    })?;
    if used != bytes.len() {
        return Err(format!(
            "synchronized registry tags have {} trailing NBT bytes",
            bytes.len() - used
        ));
    }
    let Tag::Compound(registries) = root.tag else {
        return Err("synchronized registry tags root is not a compound".into());
    };
    registries
        .into_iter()
        .map(|(registry, encoded)| {
            let registry = registry
                .to_string()
                .map_err(|_| "registry tags contain an invalid UTF-16 registry id")?;
            if !valid_resource_id(&registry) {
                return Err(format!(
                    "registry tags contain invalid registry id {registry:?}"
                ));
            }
            let Tag::Compound(tags) = encoded else {
                return Err(format!("registry tags for {registry} are not a compound"));
            };
            let tags = tags
                .into_iter()
                .map(|(name, encoded_ids)| {
                    let name = name.to_string().map_err(|_| {
                        format!("registry {registry} contains an invalid UTF-16 tag id")
                    })?;
                    if !valid_resource_id(&name) {
                        return Err(format!(
                            "registry {registry} contains invalid tag id {name:?}"
                        ));
                    }
                    let Tag::IntArray(ids) = encoded_ids else {
                        return Err(format!("tag {name} in {registry} is not an int array"));
                    };
                    if ids.iter().any(|id| *id < 0) {
                        return Err(format!(
                            "tag {name} in {registry} contains a negative entry id"
                        ));
                    }
                    Ok(TagEntry { name, ids })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(RegistryTags { registry, tags })
        })
        .collect()
}

fn registry_file(resources: &Path, registry: &str) -> PathBuf {
    let (namespace, path) = registry.split_once(':').expect("pinned registry id");
    resources
        .join("registry-sync")
        .join(namespace)
        .join(format!("{path}.nbt"))
}

fn valid_resource_id(id: &str) -> bool {
    let Some((namespace, path)) = id.split_once(':') else {
        return false;
    };
    !namespace.is_empty()
        && !path.is_empty()
        && namespace
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
        && path
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"/._-".contains(&b))
        && !path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

#[cfg(test)]
mod tests {
    use super::{RegistryBundle, SYNCHRONIZED_REGISTRIES, configuration, registry_file};
    use horizon_world::nbt::{self, JavaString, Limits, NamedTag, Tag};
    use std::collections::HashSet;
    use std::{fs, path::Path};

    #[test]
    fn pinned_vanilla_registry_sync_is_unique_and_has_world_entry_requirements() {
        let unique: HashSet<_> = SYNCHRONIZED_REGISTRIES.iter().copied().collect();
        assert_eq!(unique.len(), SYNCHRONIZED_REGISTRIES.len());
        assert_eq!(SYNCHRONIZED_REGISTRIES.len(), 32);
        assert!(unique.contains("minecraft:dimension_type"));
        assert!(unique.contains("minecraft:worldgen/biome"));
        assert!(unique.contains("minecraft:chat_type"));
        assert!(unique.contains("minecraft:damage_type"));
    }

    #[test]
    fn registry_bundle_loads_typed_nbt_in_vanilla_order() {
        let root =
            std::env::temp_dir().join(format!("borizon-registry-sync-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (index, registry) in SYNCHRONIZED_REGISTRIES.iter().enumerate() {
            let file = registry_file(&root, registry);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let entries = Tag::compound([(
                JavaString(format!("minecraft:entry_{index}").encode_utf16().collect()),
                Tag::compound([
                    ("_borizon_data_present", Tag::Byte(1)),
                    ("data", Tag::compound([("value", Tag::Int(index as i32))])),
                ]),
            )]);
            let bytes = nbt::write_named(
                &NamedTag {
                    name: JavaString::default(),
                    tag: entries,
                },
                Limits::default(),
            )
            .unwrap();
            fs::write(file, bytes).unwrap();
        }

        write_test_tags(Path::new(&root));
        write_test_order(Path::new(&root), |index| {
            vec![format!("minecraft:entry_{index}")]
        });
        let bundle = RegistryBundle::load(Path::new(&root)).unwrap();
        assert_eq!(bundle.registries().len(), SYNCHRONIZED_REGISTRIES.len());
        assert_eq!(bundle.registries()[0].registry, SYNCHRONIZED_REGISTRIES[0]);
        assert_eq!(bundle.registries()[18].registry, "minecraft:dimension_type");
        assert_eq!(bundle.registries()[18].entries[0].id, "minecraft:entry_18");
        assert_eq!(
            bundle.registries()[18].entries[0]
                .data
                .as_ref()
                .unwrap()
                .get("value"),
            Some(&Tag::Int(18))
        );
        let packets: Vec<_> = bundle.encode_packets().collect();
        assert_eq!(packets.len(), SYNCHRONIZED_REGISTRIES.len());
        for (index, packet) in packets.iter().enumerate() {
            let id = i32::from(packet[0]);
            assert_eq!(
                id,
                horizon_protocol::ids::configuration::clientbound::REGISTRY_DATA
            );
            let decoded = configuration::Clientbound::decode(id, &packet[1..]).unwrap();
            let configuration::Clientbound::RegistryData(registry) = decoded else {
                panic!("expected registry data packet");
            };
            assert_eq!(registry.registry, SYNCHRONIZED_REGISTRIES[index]);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_bundle_preserves_entries_with_data_known_from_a_pack() {
        let root = std::env::temp_dir().join(format!(
            "borizon-registry-sync-known-pack-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        for registry in SYNCHRONIZED_REGISTRIES {
            let file = registry_file(&root, registry);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let entries = Tag::compound([
                (
                    JavaString("minecraft:known".encode_utf16().collect()),
                    Tag::compound([("_borizon_data_present", Tag::Byte(0))]),
                ),
                (
                    JavaString("minecraft:with_data".encode_utf16().collect()),
                    Tag::compound([
                        ("_borizon_data_present", Tag::Byte(1)),
                        ("data", Tag::compound([("value", Tag::Int(7))])),
                    ]),
                ),
            ]);
            let bytes = nbt::write_named(
                &NamedTag {
                    name: JavaString::default(),
                    tag: entries,
                },
                Limits::default(),
            )
            .unwrap();
            fs::write(file, bytes).unwrap();
        }

        write_test_tags(Path::new(&root));
        write_test_order(Path::new(&root), |_| {
            vec!["minecraft:known".into(), "minecraft:with_data".into()]
        });
        let bundle = RegistryBundle::load(Path::new(&root)).unwrap();
        let entries = &bundle.registries()[0].entries;
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .any(|entry| entry.id == "minecraft:known" && entry.data.is_none())
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.id == "minecraft:with_data" && entry.data.is_some())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configuration_exchange_has_vanilla_packet_order_and_finish_marker() {
        let root = std::env::temp_dir().join(format!(
            "borizon-registry-sync-packet-order-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        for (index, registry) in SYNCHRONIZED_REGISTRIES.iter().enumerate() {
            let file = registry_file(&root, registry);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let entries = Tag::compound([(
                JavaString("minecraft:entry".encode_utf16().collect()),
                Tag::compound([
                    ("_borizon_data_present", Tag::Byte(1)),
                    ("data", Tag::compound([("value", Tag::Int(index as i32))])),
                ]),
            )]);
            let bytes = nbt::write_named(
                &NamedTag {
                    name: JavaString::default(),
                    tag: entries,
                },
                Limits::default(),
            )
            .unwrap();
            fs::write(file, bytes).unwrap();
        }
        write_test_tags(Path::new(&root));
        write_test_order(Path::new(&root), |_| vec!["minecraft:entry".into()]);
        let bundle = RegistryBundle::load(Path::new(&root)).unwrap();
        let packets = bundle.encode_configuration_packets();
        assert_eq!(packets.len(), SYNCHRONIZED_REGISTRIES.len() + 4);
        let decode = |packet: &[u8]| {
            let id = i32::from(packet[0]);
            configuration::Clientbound::decode(id, &packet[1..]).unwrap()
        };
        assert!(matches!(
            decode(&packets[0]),
            configuration::Clientbound::UpdateEnabledFeatures(_)
        ));
        assert!(matches!(
            decode(&packets[1]),
            configuration::Clientbound::SelectKnownPacks(_)
        ));
        for (index, packet) in packets[2..2 + SYNCHRONIZED_REGISTRIES.len()]
            .iter()
            .enumerate()
        {
            let configuration::Clientbound::RegistryData(registry) = decode(packet) else {
                panic!("expected registry packet at position {index}");
            };
            assert_eq!(registry.registry, SYNCHRONIZED_REGISTRIES[index]);
        }
        assert!(matches!(
            decode(&packets[packets.len() - 2]),
            configuration::Clientbound::UpdateTags(configuration::UpdateTags { registries })
                if registries.len() == 1 && registries[0].tags[0].ids == [0, 1, 4]
        ));
        assert!(matches!(
            decode(packets.last().unwrap()),
            configuration::Clientbound::FinishConfiguration(_)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_bundle_rejects_missing_or_malformed_entries() {
        let root =
            std::env::temp_dir().join(format!("borizon-registry-sync-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let first = registry_file(&root, SYNCHRONIZED_REGISTRIES[0]);
        fs::create_dir_all(first.parent().unwrap()).unwrap();
        fs::write(&first, b"bad nbt").unwrap();
        write_test_order(Path::new(&root), |_| vec!["minecraft:entry".into()]);
        let error = RegistryBundle::load(Path::new(&root)).unwrap_err();
        assert!(error.contains("worldgen/biome"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_entry_ids_follow_vanilla_order_not_sorted_nbt_keys() {
        use super::decode_ordered_entries;

        let encoded = Tag::compound([
            (
                "minecraft:a",
                Tag::compound([("_borizon_data_present", Tag::Byte(0))]),
            ),
            (
                "minecraft:z",
                Tag::compound([("_borizon_data_present", Tag::Byte(0))]),
            ),
        ]);
        let Tag::Compound(encoded) = encoded else {
            unreachable!();
        };
        let order = serde_json::json!(["minecraft:z", "minecraft:a"]);
        let decoded =
            decode_ordered_entries("minecraft:test", order.as_array().unwrap(), encoded).unwrap();
        assert_eq!(
            decoded
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["minecraft:z", "minecraft:a"]
        );
    }

    fn write_test_tags(root: &Path) {
        let bytes = nbt::write_named(
            &NamedTag {
                name: JavaString::default(),
                tag: Tag::compound([(
                    "minecraft:block",
                    Tag::compound([("minecraft:test", Tag::IntArray(vec![0, 1, 4]))]),
                )]),
            },
            Limits::default(),
        )
        .unwrap();
        fs::write(root.join("registry-tags.nbt"), bytes).unwrap();
    }

    fn write_test_order(root: &Path, ids_for: impl Fn(usize) -> Vec<String>) {
        let mut order = serde_json::Map::new();
        for (index, registry) in SYNCHRONIZED_REGISTRIES.iter().enumerate() {
            order.insert((*registry).into(), serde_json::json!(ids_for(index)));
        }
        fs::write(
            root.join("registry-order.json"),
            serde_json::to_vec(&order).unwrap(),
        )
        .unwrap();
    }
}
