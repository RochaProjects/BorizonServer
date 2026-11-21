//! Bulk block edits: runs of blocks up columns written straight into the
//! resident sections, a section unpacked and packed once however many of its
//! blocks change, instead of a `World::set` per block. Nothing round them is
//! told (as `fill ... strict`); the light of each chunk changed is worked out
//! again whole, off the game thread, rather than block by block, and each
//! section changed is remeshed once.
use crate::chunk::index;
use crate::fast_hash::FxHashMap;
use crate::world::World;

/// A run of one column, resolved: blocks `y0..=y1` of column (x, z) set to
/// `state` where filter `only` (if any) passes the block there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    pub x: i32,
    pub z: i32,
    pub y0: i32,
    pub y1: i32,
    pub state: u32,
    pub only: Option<u16>,
}

/// What an edit changed.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Blocks changed.
    pub changed: u64,
    /// Chunks with a change.
    pub chunks: usize,
    /// Changes of the states `tracked` asked for (block entities, watched
    /// blocks): position, previous and new state.
    pub tracked: Vec<((i32, i32, i32), u32, u32)>,
}

/// Per state answers, worked out once each.
struct Memo(Vec<u8>);
impl Memo {
    #[inline]
    fn get(&mut self, state: u32, ask: impl FnOnce(u32) -> bool) -> bool {
        let i = state as usize;
        if i >= self.0.len() {
            self.0.resize(i + 1, 0);
        }
        if self.0[i] == 0 {
            self.0[i] = 1 + u8::from(ask(state));
        }
        self.0[i] == 2
    }
}

impl World {
    /// Make `runs` in order, each seeing the changes of those before it.
    /// `matches(filter, state)` says whether a filter passes a state, and
    /// `tracked(state)` whether its changes are reported; each is asked once
    /// per state. Runs in unloaded chunks and the parts of runs outside each
    /// chunk's own column (`Chunk::layout`) are left out.
    ///
    /// Unlike `World::set`, this records nothing per block: no
    /// `block_entities.state_changed` (a caller adds or removes block
    /// entities itself, from `Outcome::tracked`), no `watched_writes` (ask
    /// for those states through `tracked`), no `block_log` entry (the chunk
    /// goes in `bulk_log` instead, sent whole), no `path_writes` (mobs'
    /// paths are not recomputed for these blocks) and no `light_edits` (the
    /// chunk and its neighbours are lit again whole, through `dirty_light`).
    pub fn edit_runs(
        &mut self,
        runs: &[Run],
        mut matches: impl FnMut(u16, u32) -> bool,
        mut tracked: impl FnMut(u32) -> bool,
    ) -> Outcome {
        let mut out = Outcome::default();
        let mut by_chunk: FxHashMap<(i32, i32), Vec<u32>> = FxHashMap::default();
        let mut order = Vec::new();
        for (i, r) in runs.iter().enumerate() {
            if r.y1 < r.y0 {
                continue;
            }
            let key = (r.x >> 4, r.z >> 4);
            by_chunk
                .entry(key)
                .or_insert_with(|| {
                    order.push(key);
                    Vec::new()
                })
                .push(i as u32);
        }
        let mut filter_memo: Vec<Memo> = Vec::new();
        let mut tracked_memo = Memo(Vec::new());
        let mut values: Vec<Option<Box<[u32; 4096]>>> = Vec::new();
        let mut changed: Vec<bool> = Vec::new();
        for key in order {
            let Some(chunk) = self.chunks.get(&key) else {
                continue;
            };
            let (min_y, sections) = (chunk.min_y(), chunk.sections.len());
            let max_y = chunk.max_y();
            let list = &by_chunk[&key];
            changed.clear();
            changed.resize(sections, false);
            values.clear();
            values.resize_with(sections, || None);
            let mut any = 0u64;
            for &i in list {
                let r = runs[i as usize];
                let (lx, lz) = (r.x & 15, r.z & 15);
                let (y0, y1) = (r.y0.max(min_y), r.y1.min(max_y - 1));
                let mut y = y0;
                while y <= y1 {
                    let s = ((y - min_y) >> 4) as usize;
                    let top = (min_y + (s as i32 + 1) * 16 - 1).min(y1);
                    let vals = values[s].get_or_insert_with(|| {
                        let mut v = Box::new([0u32; 4096]);
                        chunk.sections[s].unpack(&mut v);
                        v
                    });
                    for yy in y..=top {
                        let at = index(lx, yy, lz);
                        let old = vals[at];
                        if old == r.state {
                            continue;
                        }
                        if let Some(f) = r.only {
                            let f = f as usize;
                            if filter_memo.len() <= f {
                                filter_memo.resize_with(f + 1, || Memo(Vec::new()));
                            }
                            if !filter_memo[f].get(old, |s| matches(f as u16, s)) {
                                continue;
                            }
                        }
                        vals[at] = r.state;
                        changed[s] = true;
                        any += 1;
                        if tracked_memo.get(old, &mut tracked)
                            || tracked_memo.get(r.state, &mut tracked)
                        {
                            out.tracked.push(((r.x, yy, r.z), old, r.state));
                        }
                    }
                    y = top + 1;
                }
            }
            if any == 0 {
                continue;
            }
            out.changed += any;
            out.chunks += 1;
            let chunk =
                std::sync::Arc::make_mut(self.chunks.get_mut(&key).expect("looked up above"));
            for s in 0..sections {
                if changed[s]
                    && let Some(v) = &values[s]
                {
                    chunk.replace_blocks(s, v);
                }
            }
            self.revision = self.revision.wrapping_add(1);
            self.modified.insert(key);
            if self.block_log.is_some() {
                self.bulk_log.insert(key);
            }
            for (s, _) in changed.iter().enumerate().filter(|c| *c.1) {
                for ds in -1..=1 {
                    let sy = s as i32 + ds;
                    if !(0..sections as i32).contains(&sy) {
                        continue;
                    }
                    for dz in -1..=1 {
                        for dx in -1..=1 {
                            self.dirty_sections.insert((key.0 + dx, sy, key.1 + dz));
                        }
                    }
                }
            }
            for dz in -1..=1 {
                for dx in -1..=1 {
                    self.dirty_light.insert((key.0 + dx, key.1 + dz));
                }
            }
        }
        out
    }
}
