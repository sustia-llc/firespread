//! Cells crossed by each stencil move, against hand-derived lists.

use firespread_spread::{STENCIL, crossing};

type Offset = (isize, isize);
type Corner = (Offset, Offset);
type Expected = (Offset, Vec<Offset>, Vec<Corner>);

fn lists(dr: isize, dc: isize) -> (Vec<Offset>, Vec<Corner>) {
    let x = crossing(dr, dc).expect("move is in the stencil");
    (x.cells().to_vec(), x.corner_pairs().to_vec())
}

// Hand-derived: the segment from (0,0) to (a,b) crosses the row boundaries
// r = k + 1/2 at s = (2k+1)/(2a) and the column boundaries c = m + 1/2 at
// s = (2m+1)/(2b); listed in order of s, a shared s is a corner.
// (1,2): c at 1/4, r at 1/2, c at 3/4 -> (0,1), (1,1); no corner.
// (1,3): c at 1/6, r and c at 1/2, c at 5/6 -> (0,1), (1,2); corner between
//        (1,1) and (0,2).
// (2,3): c at 1/6, r at 1/4, c at 1/2, r at 3/4, c at 5/6 -> (0,1), (1,1),
//        (1,2), (2,2); no corner.
// (1,1): r and c at 1/2 -> no cell; corner between (1,0) and (0,1).
// (3,1): r at 1/6, r and c at 1/2, r at 5/6 -> (1,0), (2,1); corner between
//        (2,0) and (1,1).
// (-1,3): (1,3) with rows negated -> (0,1), (-1,2); corner (-1,1), (0,2).
#[test]
fn crossed_cells_match_hand_derived_lists() {
    let expected: [Expected; 6] = [
        ((1, 2), vec![(0, 1), (1, 1)], vec![]),
        ((1, 3), vec![(0, 1), (1, 2)], vec![((1, 1), (0, 2))]),
        ((2, 3), vec![(0, 1), (1, 1), (1, 2), (2, 2)], vec![]),
        ((1, 1), vec![], vec![((1, 0), (0, 1))]),
        ((3, 1), vec![(1, 0), (2, 1)], vec![((2, 0), (1, 1))]),
        ((-1, 3), vec![(0, 1), (-1, 2)], vec![((-1, 1), (0, 2))]),
    ];
    for ((dr, dc), cells, corners) in expected {
        let (got_cells, got_corners) = lists(dr, dc);
        assert_eq!(got_cells, cells, "({dr},{dc}) crossed cells {got_cells:?}");
        assert_eq!(
            got_corners, corners,
            "({dr},{dc}) corner pairs {got_corners:?}"
        );
    }
}

#[test]
fn crossings_mirror_with_the_move() {
    for &(dr, dc) in &STENCIL {
        let (cells, corners) = lists(dr, dc);
        eprintln!("({dr:2},{dc:2}): cells {cells:?} corners {corners:?}");
        let (mc, mk) = lists(-dr, dc);
        let flip = |(r, c): Offset| (-r, c);
        assert_eq!(
            mc,
            cells.iter().copied().map(flip).collect::<Vec<_>>(),
            "({dr},{dc}) vs ({},{dc}): {mc:?}",
            -dr
        );
        assert_eq!(
            mk,
            corners
                .iter()
                .map(|&(p, q)| (flip(p), flip(q)))
                .collect::<Vec<_>>(),
            "({dr},{dc}) vs ({},{dc}) corners: {mk:?}",
            -dr
        );
    }
}

#[test]
fn moves_outside_the_stencil_have_no_crossing() {
    for (dr, dc) in [(0, 0), (0, 2), (2, 2), (3, 3), (4, 1)] {
        assert!(
            crossing(dr, dc).is_none(),
            "({dr},{dc}) returned a crossing"
        );
    }
}
