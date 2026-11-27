//! The episode store's binary format: an episode as compact bytes and back, field by field.
//!
//! - Integers are LEB128 varints; signed ones are zigzag-encoded first, so small negative numbers
//!   stay small.
//! - Times are the 8 bytes of their `f64`, so they read back exactly.
//! - Strings, lists and maps are a varint length, then their contents.
//! - An option is a 0/1 byte, then its value.
//! - An enum is one byte.
//!
//! Nothing describes itself: the reader decodes exactly the fields the writer encoded, in the same
//! order. Each type lists its fields once, in [`codec_struct`]: encoding destructures the value
//! and decoding builds a struct literal from that list. A field added to the model therefore does
//! not compile until it is listed here too. Changing the layout means a new
//! [`super::FORMAT_VERSION`].

use crate::model::{
    CompletedChange, CompletedSource, CompletedTimeline, Faction, GiverKind, GiverRef, OfferSnapshot, OfferSource, OfferedQuest, PlayerContext,
    QuestClientInfo, QuestEpisode, QuestEvent, QuestEventKind, TimedValue,
};
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;

pub trait Encode {
    fn encode(&self, out: &mut Vec<u8>);
}

pub trait Decode: Sized {
    fn decode(input: &mut Input) -> Result<Self>;
}

/// Bytes being decoded.
pub struct Input<'b> {
    bytes: &'b [u8],
}

impl<'b> Input<'b> {
    pub fn new(bytes: &'b [u8]) -> Self {
        Input { bytes }
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn take(&mut self, count: usize) -> Result<&'b [u8]> {
        if count > self.bytes.len() {
            bail!("truncated: {count} bytes needed, {} left", self.bytes.len());
        }
        let (taken, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(taken)
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// A list's length, bounded by the bytes left: every element takes at least one byte, so a
    /// corrupt length fails here instead of reserving a huge list.
    fn length(&mut self) -> Result<usize> {
        let length = usize::try_from(read_varint(self)?).context("length out of range")?;
        if length > self.bytes.len() {
            bail!("length {length} exceeds the {} bytes left", self.bytes.len());
        }
        Ok(length)
    }
}

pub fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

pub fn read_varint(input: &mut Input) -> Result<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = input.byte()?;
        let bits = u64::from(byte & 0x7f);
        if shift == 63 && bits > 1 {
            bail!("varint overflows 64 bits");
        }
        value |= bits << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    bail!("varint longer than 10 bytes")
}

impl Encode for u64 {
    fn encode(&self, out: &mut Vec<u8>) {
        write_varint(out, *self);
    }
}

impl Decode for u64 {
    fn decode(input: &mut Input) -> Result<Self> {
        read_varint(input)
    }
}

impl Encode for u32 {
    fn encode(&self, out: &mut Vec<u8>) {
        write_varint(out, u64::from(*self));
    }
}

impl Decode for u32 {
    fn decode(input: &mut Input) -> Result<Self> {
        u32::try_from(read_varint(input)?).context("u32 out of range")
    }
}

impl Encode for usize {
    fn encode(&self, out: &mut Vec<u8>) {
        write_varint(out, *self as u64);
    }
}

impl Decode for usize {
    fn decode(input: &mut Input) -> Result<Self> {
        usize::try_from(read_varint(input)?).context("usize out of range")
    }
}

impl Encode for i64 {
    fn encode(&self, out: &mut Vec<u8>) {
        write_varint(out, ((*self << 1) ^ (*self >> 63)) as u64);
    }
}

impl Decode for i64 {
    fn decode(input: &mut Input) -> Result<Self> {
        let zigzag = read_varint(input)?;
        Ok((zigzag >> 1) as i64 ^ -((zigzag & 1) as i64))
    }
}

impl Encode for f64 {
    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}

impl Decode for f64 {
    fn decode(input: &mut Input) -> Result<Self> {
        let bytes: [u8; 8] = input.take(8)?.try_into().expect("took 8 bytes");
        Ok(f64::from_le_bytes(bytes))
    }
}

impl Encode for bool {
    fn encode(&self, out: &mut Vec<u8>) {
        out.push(u8::from(*self));
    }
}

impl Decode for bool {
    fn decode(input: &mut Input) -> Result<Self> {
        match input.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            other => bail!("bool byte {other}"),
        }
    }
}

impl Encode for String {
    fn encode(&self, out: &mut Vec<u8>) {
        write_varint(out, self.len() as u64);
        out.extend_from_slice(self.as_bytes());
    }
}

impl Decode for String {
    fn decode(input: &mut Input) -> Result<Self> {
        let length = input.length()?;
        Ok(std::str::from_utf8(input.take(length)?).context("string is not UTF-8")?.to_owned())
    }
}

impl<T: Encode> Encode for Vec<T> {
    fn encode(&self, out: &mut Vec<u8>) {
        write_varint(out, self.len() as u64);
        for item in self {
            item.encode(out);
        }
    }
}

impl<T: Decode> Decode for Vec<T> {
    fn decode(input: &mut Input) -> Result<Self> {
        let length = input.length()?;
        // Exactly as long as written: no room left over from growing.
        let mut items = Vec::with_capacity(length);
        for _ in 0..length {
            items.push(T::decode(input)?);
        }
        Ok(items)
    }
}

impl<T: Encode> Encode for Option<T> {
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            None => out.push(0),
            Some(value) => {
                out.push(1);
                value.encode(out);
            }
        }
    }
}

impl<T: Decode> Decode for Option<T> {
    fn decode(input: &mut Input) -> Result<Self> {
        match input.byte()? {
            0 => Ok(None),
            1 => Ok(Some(T::decode(input)?)),
            other => bail!("option byte {other}"),
        }
    }
}

impl<K: Encode, V: Encode> Encode for BTreeMap<K, V> {
    fn encode(&self, out: &mut Vec<u8>) {
        write_varint(out, self.len() as u64);
        for (key, value) in self {
            key.encode(out);
            value.encode(out);
        }
    }
}

impl<K: Decode + Ord, V: Decode> Decode for BTreeMap<K, V> {
    fn decode(input: &mut Input) -> Result<Self> {
        let length = input.length()?;
        let mut map = BTreeMap::new();
        for _ in 0..length {
            let key = K::decode(input)?;
            map.insert(key, V::decode(input)?);
        }
        Ok(map)
    }
}

/// Both directions for a struct, from one list of its fields.
macro_rules! codec_struct {
    ($name:ident $(<$param:ident>)? { $($field:ident),* $(,)? }) => {
        impl$(<$param: Encode>)? Encode for $name$(<$param>)? {
            fn encode(&self, out: &mut Vec<u8>) {
                // Exhaustive: a field missing from the list does not compile.
                let $name { $($field),* } = self;
                $( $field.encode(out); )*
            }
        }

        impl$(<$param: Decode>)? Decode for $name$(<$param>)? {
            fn decode(input: &mut Input) -> Result<Self> {
                // Struct literal fields are evaluated in the order written: the encoding order.
                Ok($name { $($field: Decode::decode(input).context(concat!(stringify!($name), ".", stringify!($field)))?),* })
            }
        }
    };
}

/// Both directions for a fieldless enum, one byte per variant.
macro_rules! codec_enum {
    ($name:ident { $($variant:ident = $byte:literal),* $(,)? }) => {
        impl Encode for $name {
            fn encode(&self, out: &mut Vec<u8>) {
                // Exhaustive: a variant missing from the list does not compile.
                out.push(match self { $($name::$variant => $byte),* });
            }
        }

        impl Decode for $name {
            fn decode(input: &mut Input) -> Result<Self> {
                match input.byte()? {
                    $($byte => Ok($name::$variant),)*
                    other => bail!(concat!("unknown ", stringify!($name), " byte {}"), other),
                }
            }
        }
    };
}

// For the other files written in this format (ingest's spill files).
pub(crate) use codec_struct;

codec_enum!(GiverKind { Npc = 0, Object = 1, Item = 2 });
codec_enum!(Faction { Alliance = 0, Horde = 1, Neutral = 2 });
codec_enum!(CompletedSource { Modern = 0, Legacy = 1 });
codec_enum!(QuestEventKind {
    Detail = 0,
    Accepted = 1,
    Progress = 2,
    Complete = 3,
    TurnedIn = 4,
    Abandoned = 5,
    Removed = 6,
});
codec_enum!(OfferSource { Gossip = 0, Greeting = 1, Detail = 2 });

codec_struct!(TimedValue<T> { t, v });
codec_struct!(GiverRef { kind, id });
codec_struct!(PlayerContext { race_id, class_id, faction });
codec_struct!(CompletedChange { t, add, remove });
codec_struct!(CompletedTimeline { source, initial, changes });
codec_struct!(QuestEvent { t, kind, quest_id, giver });
codec_struct!(OfferedQuest {
    id,
    trivial,
    frequency,
    repeatable
});
codec_struct!(OfferSnapshot {
    t,
    source,
    giver,
    available,
    unresolved_titles,
    active,
    list_complete,
});
codec_struct!(QuestClientInfo {
    is_breadcrumb,
    quest_line_id,
    classification,
});
codec_struct!(QuestEpisode {
    key,
    contributor_id,
    character_key,
    submission_ids,
    session_name,
    session_order,
    interface_version,
    locale,
    player,
    duration,
    levels,
    completed,
    quest_log,
    quest_events,
    offers,
    quest_info,
});

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T: Encode + Decode + PartialEq + std::fmt::Debug>(value: &T) -> usize {
        let mut bytes = Vec::new();
        value.encode(&mut bytes);
        let mut input = Input::new(&bytes);
        assert_eq!(&T::decode(&mut input).unwrap(), value);
        assert!(input.is_empty(), "every byte read back");
        bytes.len()
    }

    #[test]
    fn integers_round_trip_at_their_edges_and_small_ones_take_one_byte() {
        for value in [0i64, 1, -1, 63, -64, 64, -65, i64::MAX, i64::MIN, 92514] {
            round_trip(&value);
        }
        for value in [0u64, 127, 128, u64::MAX] {
            round_trip(&value);
        }
        assert_eq!(round_trip(&-64i64), 1);
        assert_eq!(round_trip(&92514i64), 3, "a Forever quest id takes 3 bytes");
        assert_eq!(round_trip(&u32::MAX), 5);
    }

    #[test]
    fn times_keep_their_exact_bits() {
        for value in [0.0, -0.0, 0.1 + 0.2, 1e-300, f64::MAX, 3601.004999999999] {
            let mut bytes = Vec::new();
            value.encode(&mut bytes);
            let back = f64::decode(&mut Input::new(&bytes)).unwrap();
            assert_eq!(back.to_bits(), value.to_bits());
        }
    }

    fn episode() -> QuestEpisode {
        let giver = Some(GiverRef {
            kind: GiverKind::Object,
            id: 300_142,
        });
        QuestEpisode {
            key: "1790813100.5:1790813100.25".into(),
            contributor_id: "0f3a".into(),
            character_key: "0f3a:2".into(),
            submission_ids: vec!["a".into(), "b".into()],
            session_name: None,
            session_order: 7,
            interface_version: Some(16001),
            locale: Some("deDE".into()),
            player: PlayerContext {
                race_id: Some(2),
                class_id: None,
                faction: Some(Faction::Horde),
            },
            duration: 3600.5,
            levels: vec![TimedValue { t: 0.0, v: 12 }, TimedValue { t: 1800.25, v: 13 }],
            completed: Some(CompletedTimeline {
                source: CompletedSource::Legacy,
                initial: vec![1, 2, 92514],
                changes: vec![CompletedChange {
                    t: 5.0,
                    add: Some(vec![3]),
                    remove: None,
                }],
            }),
            quest_log: vec![TimedValue { t: 0.0, v: vec![] }, TimedValue { t: 9.0, v: vec![4, -5] }],
            quest_events: vec![QuestEvent {
                t: 9.0,
                kind: QuestEventKind::Accepted,
                quest_id: 4,
                giver,
            }],
            offers: vec![OfferSnapshot {
                t: 8.5,
                source: OfferSource::Greeting,
                giver,
                available: vec![OfferedQuest {
                    id: 4,
                    trivial: Some(false),
                    frequency: Some(2),
                    repeatable: None,
                }],
                unresolved_titles: Some(vec!["Ü title".into()]),
                active: Some(vec![]),
                list_complete: true,
            }],
            quest_info: Some(BTreeMap::from([(
                4,
                QuestClientInfo {
                    is_breadcrumb: Some(true),
                    quest_line_id: None,
                    classification: Some(6),
                },
            )])),
        }
    }

    #[test]
    fn an_episode_round_trips_with_every_kind_of_field_set() {
        round_trip(&episode());
    }

    #[test]
    fn an_episode_with_every_option_empty_round_trips() {
        let empty = QuestEpisode {
            session_name: None,
            interface_version: None,
            locale: None,
            player: PlayerContext::default(),
            completed: None,
            quest_info: None,
            ..episode()
        };
        round_trip(&empty);
    }

    #[test]
    fn truncated_or_corrupt_bytes_fail_instead_of_decoding_garbage() {
        let mut bytes = Vec::new();
        episode().encode(&mut bytes);
        for cut in [0, 1, bytes.len() / 2, bytes.len() - 1] {
            assert!(QuestEpisode::decode(&mut Input::new(&bytes[..cut])).is_err(), "cut at {cut}");
        }
        // A length far beyond the bytes left fails up front.
        let mut huge = Vec::new();
        write_varint(&mut huge, u64::MAX >> 1);
        assert!(Vec::<i64>::decode(&mut Input::new(&huge)).is_err());
        assert!(bool::decode(&mut Input::new(&[2])).is_err());
    }
}
