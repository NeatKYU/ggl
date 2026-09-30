//! 커밋 그래프의 레인(세로줄) 배치를 계산한다.
//!
//! 위에서 아래로 커밋을 하나씩 내려가면서
//! 1. 이 커밋을 기다리던 레인이 있으면 그중 가장 왼쪽에, 없으면 빈 레인에 놓는다.
//! 2. 첫 번째 부모는 같은 레인·같은 색을 이어받는다.
//! 3. 두 번째 이후 부모(머지)는 다른 레인으로 곡선을 그어 합류한다.
//! 4. 끝난 레인은 비워서 다시 쓴다.

/// 목록에 없는(아직 안 불러온) 부모. 선은 아래로 계속 이어진다.
pub const OFFSCREEN: usize = usize::MAX;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub lane: usize,
    pub color: usize,
}

/// `row`에서 `row + 1`로 내려가는 선 하나
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub color: usize,
    /// 커밋 안 된 변경 → HEAD 구간
    pub dashed: bool,
}

#[derive(Debug, Default)]
pub struct Layout {
    pub nodes: Vec<Node>,
    /// `edges[r]` = r번째 행에서 다음 행으로 내려가는 선들
    pub edges: Vec<Vec<Edge>>,
    pub lanes: usize,
}

#[derive(Clone, Copy)]
struct Lane {
    target: usize,
    color: usize,
    dashed: bool,
    /// 이번 행에서 머지 곡선으로 새로 시작된 레인이면 곡선의 시작 레인
    starts_from: Option<usize>,
}

/// `parents[r]` = r번째 행의 부모 행 번호들 (첫 번째 부모가 먼저). 목록에 없으면 `OFFSCREEN`.
/// `dashed_first`면 0번 행(커밋 안 된 변경)에서 첫 부모까지의 선을 점선으로 표시한다.
pub fn layout(parents: &[Vec<usize>], dashed_first: bool) -> Layout {
    let n = parents.len();
    let mut lanes: Vec<Option<Lane>> = Vec::new();
    let mut out = Layout { nodes: Vec::with_capacity(n), edges: Vec::with_capacity(n), lanes: 0 };

    for row in 0..n {
        // 1. 이 커밋을 기다리던 레인들
        let waiting: Vec<usize> = (0..lanes.len())
            .filter(|&i| lanes[i].is_some_and(|l| l.target == row))
            .collect();
        let (lane, color) = match waiting.first() {
            Some(&i) => (i, lanes[i].unwrap().color),
            None => (free_slot(&mut lanes), next_color(&lanes)),
        };
        for &i in &waiting {
            lanes[i] = None;
        }
        out.nodes.push(Node { lane, color });

        // 2~3. 부모 연결
        let mut extra = Vec::new();
        for (k, &p) in parents[row].iter().enumerate() {
            // 부모는 항상 아래 행이어야 한다. 아니면 목록 밖으로 취급.
            let p = if p != OFFSCREEN && p <= row { OFFSCREEN } else { p };
            if k == 0 {
                let dashed = dashed_first && row == 0;
                lanes[lane] = Some(Lane { target: p, color, dashed, starts_from: None });
                continue;
            }
            let existing = (p != OFFSCREEN)
                .then(|| (0..lanes.len()).find(|&i| lanes[i].is_some_and(|l| l.target == p)))
                .flatten();
            match existing {
                // 이미 그 부모로 가는 레인이 있으면 그 레인으로 합류하는 선만 긋는다.
                Some(i) => extra.push((i, lanes[i].unwrap().color)),
                None => {
                    let slot = free_slot(&mut lanes);
                    let color = next_color(&lanes);
                    lanes[slot] = Some(Lane { target: p, color, dashed: false, starts_from: Some(lane) });
                }
            }
        }

        // 4. 다음 행으로 내려가는 선. 다음 커밋을 기다리는 레인들은 가장 왼쪽 레인으로 모인다.
        let next = row + 1;
        let join = (0..lanes.len()).find(|&i| lanes[i].is_some_and(|l| l.target == next));
        let dest = |i: usize, l: &Lane| if l.target == next { join.unwrap_or(i) } else { i };
        let mut edges = Vec::new();
        for (i, l) in lanes.iter_mut().enumerate() {
            let Some(l) = l else { continue };
            let from = l.starts_from.take().unwrap_or(i);
            edges.push(Edge { from, to: dest(i, l), color: l.color, dashed: l.dashed });
        }
        for (i, color) in extra {
            edges.push(Edge { from: lane, to: dest(i, &lanes[i].unwrap()), color, dashed: false });
        }
        out.edges.push(edges);

        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        out.lanes = out.lanes.max(lanes.len()).max(lane + 1);
    }
    out
}

fn free_slot(lanes: &mut Vec<Option<Lane>>) -> usize {
    match lanes.iter().position(Option::is_none) {
        Some(i) => i,
        None => {
            lanes.push(None);
            lanes.len() - 1
        }
    }
}

/// 지금 쓰고 있지 않은 색 중 가장 앞 번호
fn next_color(lanes: &[Option<Lane>]) -> usize {
    (0..).find(|c| !lanes.iter().flatten().any(|l| l.color == *c)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_line() {
        let l = layout(&[vec![1], vec![2], vec![]], false);
        assert!(l.nodes.iter().all(|n| n.lane == 0 && n.color == 0));
        assert_eq!(l.edges[0], vec![Edge { from: 0, to: 0, color: 0, dashed: false }]);
        assert!(l.edges[2].is_empty());
        assert_eq!(l.lanes, 1);
    }

    #[test]
    fn branch_and_merge() {
        // 0: merge(1, 2)   1: main   2: feature   3: base
        let l = layout(&[vec![1, 2], vec![3], vec![3], vec![]], false);
        assert_eq!(l.nodes[0].lane, 0);
        assert_eq!(l.nodes[1].lane, 0);
        assert_eq!(l.nodes[2].lane, 1);
        assert_eq!(l.nodes[3].lane, 0);
        // 머지 곡선: 0번 레인에서 1번 레인으로
        assert!(l.edges[0].contains(&Edge { from: 0, to: 1, color: 1, dashed: false }));
        // feature 레인이 base로 합류
        assert!(l.edges[2].contains(&Edge { from: 1, to: 0, color: 1, dashed: false }));
        assert_eq!(l.lanes, 2);
    }

    #[test]
    fn two_tips_fork() {
        // 0: tip A   1: tip B   2: 공통 부모
        let l = layout(&[vec![2], vec![2], vec![]], false);
        assert_eq!(l.nodes[0].lane, 0);
        assert_eq!(l.nodes[1].lane, 1);
        assert_ne!(l.nodes[0].color, l.nodes[1].color);
        assert_eq!(l.nodes[2].lane, 0);
        assert!(l.edges[1].contains(&Edge { from: 1, to: 0, color: l.nodes[1].color, dashed: false }));
    }

    #[test]
    fn uncommitted_lane_is_dashed_until_head() {
        // 0: 커밋 안 된 변경(→2)  1: 다른 브랜치 팁(→2)  2: HEAD(→3)  3
        let l = layout(&[vec![2], vec![2], vec![3], vec![]], true);
        assert!(l.edges[0].iter().all(|e| e.dashed));
        assert!(l.edges[1].iter().any(|e| e.from == 0 && e.dashed));
        assert!(l.edges[1].iter().any(|e| e.from == 1 && !e.dashed));
        assert!(l.edges[2].iter().all(|e| !e.dashed));
    }

    #[test]
    fn offscreen_parent_continues() {
        let l = layout(&[vec![OFFSCREEN]], false);
        assert_eq!(l.edges[0], vec![Edge { from: 0, to: 0, color: 0, dashed: false }]);
    }

    #[test]
    fn freed_lane_is_reused() {
        // 0: A(→2)  1: B(→2)  2: C(→3)  3: D(끝)  4: 새 팁 → 비워진 0번 레인을 다시 쓴다
        let l = layout(&[vec![2], vec![2], vec![3], vec![], vec![]], false);
        assert_eq!(l.nodes[4].lane, 0);
        assert_eq!(l.lanes, 2);
    }
}
