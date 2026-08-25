use anyhow::{Context, Result};
use std::fs;

pub const SLABINFO: &str = "/proc/slabinfo";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlabCache {
    pub name: String,
    pub active_objs: u64,
    pub num_objs: u64,
    pub obj_size: u64,
    pub objs_per_slab: u64,
    pub pages_per_slab: u64,
    pub active_slabs: u64,
    pub num_slabs: u64,
}

impl SlabCache {
    /// Bytes occupied by all slabs of this cache (allocated, not only live objects).
    pub fn size_bytes(&self) -> u64 {
        self.num_slabs
            .saturating_mul(self.pages_per_slab)
            .saturating_mul(crate::zram::page_size())
    }

    /// Approximate bytes held by live objects.
    pub fn active_bytes(&self) -> u64 {
        self.active_objs.saturating_mul(self.obj_size)
    }

    /// Allocated-but-unused bytes: fragmentation inside partially filled slabs.
    pub fn waste_bytes(&self) -> u64 {
        self.size_bytes().saturating_sub(self.active_bytes())
    }
}

pub fn read_slabinfo() -> Result<Vec<SlabCache>> {
    let text = fs::read_to_string(SLABINFO)
        .with_context(|| format!("read {SLABINFO}; most kernels restrict it to root"))?;
    Ok(parse_slabinfo(&text))
}

/// Parse `/proc/slabinfo`. Canonical per-cache line shape (see `mm/slab_common.c`):
/// `name active_objs num_objs objsize objperslab pagesperslab : tunables .. : slabdata active_slabs num_slabs sharedavail`
///
/// Header/comment/malformed lines are skipped. Cache names containing ':'
/// are handled by splitting on the canonical `" : tunables"` separator.
pub fn parse_slabinfo(text: &str) -> Vec<SlabCache> {
    let mut caches = Vec::new();
    for line in text.lines() {
        // The kernel always separates sections with " : tunables" and
        // " : slabdata"; fall back to the whole line when absent.
        let head = match line.split_once(" : tunables") {
            Some((head, _)) => head,
            None => line,
        };
        let mut tokens = head.split_whitespace();
        let Some(name) = tokens.next() else {
            continue;
        };
        if name.starts_with('#') || name == "slabinfo" {
            continue;
        }
        let numbers: Vec<u64> = tokens.filter_map(|token| token.parse().ok()).collect();
        let [active_objs, num_objs, obj_size, objs_per_slab, pages_per_slab] = numbers[..] else {
            continue;
        };

        let (active_slabs, num_slabs) = match line.split_once("slabdata") {
            Some((_, tail)) => {
                let mut values = tail
                    .split_whitespace()
                    .filter_map(|token| token.parse::<u64>().ok());
                match (values.next(), values.next()) {
                    (Some(active), Some(total)) => (active, total),
                    _ => derive_slab_counts(numbers[0], numbers[1], objs_per_slab),
                }
            }
            None => derive_slab_counts(numbers[0], numbers[1], objs_per_slab),
        };

        caches.push(SlabCache {
            name: name.to_string(),
            active_objs,
            num_objs,
            obj_size,
            objs_per_slab,
            pages_per_slab,
            active_slabs,
            num_slabs,
        });
    }
    caches
}

fn derive_slab_counts(active_objs: u64, num_objs: u64, objs_per_slab: u64) -> (u64, u64) {
    let per_slab = objs_per_slab.max(1);
    (active_objs / per_slab, num_objs.div_ceil(per_slab))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
slabinfo - version: 2.1
# name            <active_objs> <num_objs> <objsize> <objperslab> <pagesperslab> : tunables <limit> <batchcount> <sharedfactor> : slabdata <active_slabs> <num_slabs> <sharedavail>
nf_conntrack      123   140    320   25   2 : tunables    0   0   0 : slabdata     5     5      0
dentry             98765 100000  192   21   1 : tunables    0   0   0 : slabdata  4762  4762      0
kmalloc-4k         42     60   4096    8   8 : tunables    0   0   0 : slabdata     8     8      0
";

    #[test]
    fn parses_slabinfo_sample() {
        let caches = parse_slabinfo(SAMPLE);
        assert_eq!(caches.len(), 3);

        let dentry = caches.iter().find(|c| c.name == "dentry").unwrap();
        assert_eq!(dentry.active_objs, 98765);
        assert_eq!(dentry.num_objs, 100000);
        assert_eq!(dentry.obj_size, 192);
        assert_eq!(dentry.pages_per_slab, 1);
        assert_eq!(dentry.objs_per_slab, 21);
        assert_eq!(dentry.active_slabs, 4762);
        assert_eq!(dentry.num_slabs, 4762);
        assert_eq!(dentry.size_bytes(), 4762 * 4096);
        assert_eq!(dentry.active_bytes(), 98765 * 192);

        let conntrack = caches.iter().find(|c| c.name == "nf_conntrack").unwrap();
        assert_eq!(conntrack.size_bytes(), 5 * 2 * 4096);
        assert_eq!(
            caches
                .iter()
                .find(|c| c.name == "kmalloc-4k")
                .unwrap()
                .waste_bytes(),
            8 * 8 * 4096 - 42 * 4096
        );
    }

    #[test]
    fn skips_malformed_lines() {
        assert!(parse_slabinfo("# comment\n\nbroken line\n").is_empty());
        assert!(parse_slabinfo("").is_empty());
        assert!(parse_slabinfo("slabinfo - version: 2.1\n").is_empty());
    }

    #[test]
    fn keeps_cache_names_containing_colons() {
        let caches =
            parse_slabinfo("xfs_buf:meta 10 20 256 4 1 : tunables 0 0 0 : slabdata 5 5 0\n");
        assert_eq!(caches.len(), 1);
        assert_eq!(caches[0].name, "xfs_buf:meta");
        assert_eq!(caches[0].num_slabs, 5);
    }

    #[test]
    fn derives_slab_counts_without_slabdata_section() {
        let caches = parse_slabinfo("weirdcache 10 20 256 4 1 : tunables 0 0 0 :\n");
        assert_eq!(caches.len(), 1);
        let cache = &caches[0];
        assert_eq!(cache.active_slabs, 2);
        assert_eq!(cache.num_slabs, 5);
        assert_eq!(cache.waste_bytes(), 5 * 4096 - 10 * 256);
    }

    #[test]
    fn tolerates_zero_objs_per_slab_in_derivation() {
        let (active, total) = derive_slab_counts(7, 7, 0);
        assert_eq!((active, total), (7, 7));
    }
}
