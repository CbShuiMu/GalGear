//! 对照已匹配的脚本坐标与图中节点的来源。出边的目标标签不能充当节点自身的位置。
pub(super) struct Anchor {
    pub script: String,
    pub scene: String,
    pub index: usize,
    pub coordinate: Coordinate,
}

pub(super) enum Coordinate {
    Psb(usize),
    Siglus { opening: String },
}

impl Anchor {
    pub(super) fn belongs_to(&self, pc: &str, evidence: &str) -> bool {
        match &self.coordinate {
            Coordinate::Psb(index) => psb_node(&self.script, &self.scene, *index, pc, evidence),
            Coordinate::Siglus { opening } => siglus_node(&self.script, opening, pc, evidence),
        }
    }
}

fn psb_node(script: &str, scene: &str, index: usize, pc: &str, evidence: &str) -> bool {
    fn coordinate<'a>(field: &'a str, script: &str) -> Option<&'a str> {
        let (source, rest) = field.split_once(char::is_whitespace)?;
        source
            .eq_ignore_ascii_case(script)
            .then(|| rest.trim_start())
    }

    let pc_coordinate = coordinate(pc, script);
    let evidence_coordinate = coordinate(evidence, script);
    // 没有文件名的 scenes[i] 只在证据首部确认了同一脚本时使用。
    let range = pc_coordinate
        .or_else(|| evidence_coordinate.map(|_| pc))
        .and_then(|s| s.strip_prefix("scenes["))
        .and_then(|s| s.split_once(']'));
    if let Some((range, _)) = range {
        let (lo, hi) = range.split_once('-').unwrap_or((range, range));
        return lo
            .parse::<usize>()
            .ok()
            .zip(hi.parse::<usize>().ok())
            .is_some_and(|(lo, hi)| (lo..=hi).contains(&index));
    }
    for field in [pc_coordinate, evidence_coordinate].into_iter().flatten() {
        if field.starts_with('*') {
            return field.strip_prefix(scene).is_some_and(|rest| {
                rest.chars()
                    .next()
                    .is_none_or(|c| !c.is_alphanumeric() && c != '_' && c != '-')
            });
        }
    }
    if pc.eq_ignore_ascii_case(script) {
        return true;
    }
    let Some(series) = pc.strip_suffix(".ks.scn") else {
        return false;
    };
    let Some(stem) = script.strip_suffix(".ks.scn") else {
        return false;
    };
    series
        .split('/')
        .any(|part| part == stem || numbered_range_contains(part, stem))
}

// 兼容旧图中的 b111-b114 这种同前缀、等宽度的整章编号区间。
fn numbered_range_contains(range: &str, stem: &str) -> bool {
    let Some((lo, hi)) = range.split_once('-') else {
        return false;
    };
    fn parts(s: &str) -> Option<(&str, &str)> {
        let (prefix, digits) = s.split_at(s.find(|c: char| c.is_ascii_digit())?);
        digits
            .bytes()
            .all(|b| b.is_ascii_digit())
            .then_some((prefix, digits))
    }
    let (Some((a, first)), Some((b, last)), Some((c, current))) =
        (parts(lo), parts(hi), parts(stem))
    else {
        return false;
    };
    a == b
        && b == c
        && first.len() == last.len()
        && last.len() == current.len()
        && (first..=last).contains(&current)
}

fn siglus_node(script: &str, opening: &str, pc: &str, evidence: &str) -> bool {
    let Some(stem) = script.strip_suffix(".dat") else {
        return false;
    };
    if pc.eq_ignore_ascii_case(script) {
        return true;
    }
    // 旧图将指向合并章节的 farcall 列在节点证据里。仅认完整的调用标识符，
    // 省略公共前缀的简写不作猜测。带行号的单点坐标也不代表整段脚本。
    opening.chars().count() >= 4
        && evidence.contains(opening)
        && evidence
            .split_once("farcall ")
            .and_then(|(_, calls)| calls.split_whitespace().next())
            .is_some_and(|calls| {
                calls
                    .split(['；', ';'])
                    .next()
                    .unwrap_or_default()
                    .split(['/', '~', '、', ','])
                    .any(|target| target.eq_ignore_ascii_case(stem))
            })
}
