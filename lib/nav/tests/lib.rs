use math::{Pos, Size, Tiles};
use nav::{Grid, astar};

fn at(x: f32, y: f32) -> Pos<Tiles> {
    Pos::new(Tiles(x), Tiles(y))
}

fn open(width: usize, height: usize) -> Grid<Tiles> {
    Grid::new(
        Size::new(Tiles(width as f32), Tiles(height as f32)),
        vec![true; width * height],
    )
}

#[test]
fn straight_path_is_endpoints() {
    let grid = open(5, 1);
    let path = astar(&grid, at(0.0, 0.0), at(4.0, 0.0)).expect("a path across open ground");
    assert_eq!(path.first(), Some(&at(0.0, 0.0)));
    assert_eq!(path.last(), Some(&at(4.0, 0.0)));
}

#[test]
fn routes_around_a_wall() {
    let (width, height) = (3, 3);
    let mut cells = vec![true; width * height];
    cells[1] = false;
    cells[width + 1] = false;
    let grid = Grid::new(Size::new(Tiles(width as f32), Tiles(height as f32)), cells);
    assert!(astar(&grid, at(0.0, 0.0), at(2.0, 0.0)).is_some());
}

#[test]
fn diagonal_steps_may_pass_blocked_corners() {
    let mut cells = vec![true; 9];
    cells[1] = false;
    cells[3] = false;
    let grid = Grid::new(Size::new(Tiles(3.0), Tiles(3.0)), cells);
    let path = astar(&grid, at(0.0, 0.0), at(1.0, 1.0)).expect("diagonal between blocked corners");
    assert_eq!(path, vec![at(0.0, 0.0), at(1.0, 1.0)]);
}

#[test]
fn nearest_walkable_snaps_to_open() {
    let mut cells = vec![false; 9];
    cells[8] = true;
    let grid = Grid::new(Size::new(Tiles(3.0), Tiles(3.0)), cells);
    assert_eq!(grid.nearest_walkable(at(0.0, 0.0)), Some(at(2.0, 2.0)));
}
