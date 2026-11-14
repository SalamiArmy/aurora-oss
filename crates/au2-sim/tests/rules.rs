//! One or more tests per rule in RULES.md. Maps are built in code so each test states exactly what it relies on.

use au2_map::*;
use au2_sim::math::Vec2;
use au2_sim::rules::*;
use au2_sim::*;

// ---------------------------------------------------------------- builders
fn sdef(f: i32, x: f64, y: f64, max: i32, units: u32) -> SunDef {
    SunDef { faction: f, health: 100, growth: 0, power: if f >= 0 { 1 } else { 0 }, max_power: max, starting_units: if f >= 0 { units } else { 0 }, awareness_radius: -1.0, pos: [x, y] }
}
fn sun(f: i32, x: f64, y: f64, max: i32, units: u32) -> Object {
    Object::Sun(sdef(f, x, y, max, units))
}
fn map(n: usize, objs: Vec<Object>) -> Map {
    let factions = (0..n)
        .map(|i| FactionDef { id: ["fi_player", "fi_player2", "fi_player3", "fi_player4"][i].into(), think_period: 0, max_sources: 8, max_orders: 8 })
        .collect();
    let lines = vec![1; objs.len()];
    Map { factions, objects: objs, object_lines: lines }
}
fn sim(objs: Vec<Object>) -> Sim {
    Sim::new(&map(2, objs), SimConfig { seed: 7, ai_difficulty: 0, human_ai: false, enable_ai: false })
}
fn run(s: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        s.step(&[]);
    }
}
fn order(f: u8, src: usize, t: Target, pct: u8) -> Command {
    Command::Order { faction: f, sources: vec![src], target: t, then: None, percent: pct, repeat: false }
}
fn ship(f: u8, x: f64, y: f64, target: Target) -> Ship {
    // heading towards the origin (where most scenario targets are), else east
    Ship { id: 0, pos: Vec2::new(x, y), heading: Vec2::new(-x, -y).norm_or(Vec2::new(1.0, 0.0)), push: Vec2::ZERO, faction: f, target, then: None, speed_mul: 1.0, cooldown: 0, last_wormhole: 0, alive: true, neutral: false }
}
fn s(sim: &Sim, i: usize) -> &au2_sim::Sun {
    sim.sun(i).unwrap()
}
fn terrain(kind: TerrainKind, x: f64, y: f64, rows: u32, cols: u32, tile: f64) -> TerrainDef {
    TerrainDef {
        kind,
        data: vec![1; (rows * cols) as usize],
        tile_size: tile,
        rows,
        cols,
        outer_radius: 0.0,
        inner_radius: 0.0,
        switch_time: 0,
        enabled: true,
        pos: [x, y],
        acceleration: [0.0, 0.0],
        dir_switch: 0,
        speed_change: 0.0,
        death_rate: 0.0,
        density: 100.0,
        faction_change_rate: 0.0,
        lose_order_rate: 0.0,
    }
}
// two far-apart home suns used as the base of most scenarios (so nobody is eliminated)
fn base() -> Vec<Object> {
    vec![sun(0, -9000.0, 9000.0, 3, 100), sun(1, 9000.0, 9000.0, 3, 100)]
}
fn with(mut b: Vec<Object>, extra: Vec<Object>) -> Vec<Object> {
    b.extend(extra);
    b
}

// ---------------------------------------------------------------- 2. suns: production
#[test]
fn level_one_sun_makes_about_one_ship_per_second() {
    let mut sm = sim(base());
    run(&mut sm, 600);
    assert_eq!(s(&sm, 0).stock, 110);
}

#[test]
fn production_grows_with_level_and_level_five_is_several_times_level_one() {
    assert_eq!(production_per_sec(0), 0.0);
    assert_eq!(production_per_sec(1), 1.0);
    for l in 1..5 {
        assert!(production_per_sec(l + 1) > production_per_sec(l));
    }
    assert!(production_per_sec(5) >= 4.0 * production_per_sec(1));
}

#[test]
fn level_zero_sun_does_not_produce() {
    let mut sm = sim(base());
    if let Obj::Sun(x) = &mut sm.objs[0] {
        x.level = 0;
    }
    run(&mut sm, 600);
    assert_eq!(s(&sm, 0).stock, 100);
}

#[test]
fn neutral_suns_produce_nothing() {
    let mut sm = sim(with(base(), vec![sun(-1, 0.0, 0.0, 3, 0)]));
    run(&mut sm, 600);
    assert_eq!(s(&sm, 2).stock, 0);
    assert_eq!(s(&sm, 2).faction, -1);
}

#[test]
fn starting_state_is_level_one_with_initial_stock() {
    let sm = sim(base());
    assert_eq!((s(&sm, 0).level, s(&sm, 0).stock), (1, 100));
}

#[test]
fn bigger_maximum_means_bigger_radius() {
    assert!(sun_radius(2) > sun_radius(1));
    assert!(sun_radius(3) > sun_radius(2));
    assert!(sun_radius(5) > sun_radius(3));
}

// ---------------------------------------------------------------- 2. suns: upgrading
#[test]
fn friendly_ships_upgrade_a_sun_and_advance_one_level() {
    let mut sm = sim(base());
    // send 25 of the ~100 ships to the sun itself
    sm.issue(order(0, 0, Target::Obj(0), 25)).unwrap();
    assert_eq!(s(&sm, 0).stock, 75);
    run(&mut sm, 3);
    assert_eq!(s(&sm, 0).level, 2, "20 ships fill level 1 -> 2");
    assert_eq!(s(&sm, 0).progress, 5);
}

#[test]
fn sun_at_maximum_absorbs_ships_into_reserve() {
    let mut sm = sim(vec![sun(0, 0.0, 0.0, 1, 100), sun(1, 9000.0, 9000.0, 1, 100)]);
    assert_eq!(s(&sm, 0).level, s(&sm, 0).max_level);
    sm.issue(order(0, 0, Target::Obj(0), 50)).unwrap();
    run(&mut sm, 3);
    assert_eq!(s(&sm, 0).level, 1);
    assert_eq!(s(&sm, 0).progress, 0);
    assert!(s(&sm, 0).stock >= 100, "ships are not wasted: stock {}", s(&sm, 0).stock);
}

// ---------------------------------------------------------------- 2. suns: damage, capture
#[test]
fn hostile_ships_damage_a_sun_until_it_loses_levels_then_goes_neutral() {
    let mut sm = sim(vec![sun(0, -9000.0, 9000.0, 3, 100), sun(1, 0.0, 0.0, 3, 0)]);
    if let Obj::Sun(x) = &mut sm.objs[1] {
        x.level = 2;
        x.progress = 0;
    }
    // faction 0 ships right next to sun 1 aiming at it
    for i in 0..5 {
        sm.ships.push(ship(0, 300.0 + i as f64, 0.0, Target::Obj(1)));
    }
    sm.step(&[]);
    // first damage drops progress below zero -> level 1
    assert_eq!(s(&sm, 1).level, 1);
    assert_eq!(s(&sm, 1).faction, 1);
    for i in 0..200 {
        sm.ships.push(ship(0, 300.0 + i as f64 * 0.01, 5.0, Target::Obj(1)));
    }
    sm.step(&[]);
    assert_ne!(s(&sm, 1).faction, 1, "defender must lose the sun");
}

#[test]
fn level_zero_sun_goes_neutral_on_next_hit() {
    let mut sm = sim(vec![sun(0, -9000.0, 9000.0, 3, 100), sun(1, 0.0, 0.0, 3, 0)]);
    if let Obj::Sun(x) = &mut sm.objs[1] {
        x.level = 0;
        x.progress = 0;
    }
    sm.ships.push(ship(0, 300.0, 0.0, Target::Obj(1)));
    sm.step(&[]);
    assert_eq!(s(&sm, 1).faction, -1);
    assert_eq!(s(&sm, 1).level, 0);
}

#[test]
fn garrison_softens_hits() {
    let mut a = sim(vec![sun(0, -9000.0, 9000.0, 3, 100), sun(1, 0.0, 0.0, 3, 0)]);
    let mut b = sim(vec![sun(0, -9000.0, 9000.0, 3, 100), sun(1, 0.0, 0.0, 3, 0)]);
    for sm in [&mut a, &mut b] {
        if let Obj::Sun(x) = &mut sm.objs[1] {
            x.level = 2;
            x.progress = 10;
        }
    }
    if let Obj::Sun(x) = &mut b.objs[1] {
        x.stock = 50;
    }
    for sm in [&mut a, &mut b] {
        for i in 0..6 {
            sm.ships.push(ship(0, 300.0 + i as f64, 0.0, Target::Obj(1)));
        }
        sm.step(&[]);
    }
    let health = |x: &Sim| (s(x, 1).level, s(x, 1).progress);
    assert!(health(&b) > health(&a), "garrisoned {:?} vs bare {:?}", health(&b), health(&a));
}

#[test]
fn ships_capture_a_neutral_sun_for_themselves() {
    let mut sm = sim(with(base(), vec![sun(-1, 0.0, 0.0, 2, 0)]));
    // 12 of 100 ships, flying from far away
    sm.issue(order(0, 0, Target::Obj(2), 12)).unwrap();
    for _ in 0..2000 {
        sm.step(&[]);
        if s(&sm, 2).faction == 0 {
            break;
        }
    }
    assert_eq!(s(&sm, 2).faction, 0);
    assert_eq!(s(&sm, 2).level, 0, "a captured sun starts undeveloped");
    run(&mut sm, 300);
    assert_eq!(s(&sm, 2).stock, 0, "level 0 produces nothing");
}

#[test]
fn contested_capture_needs_more_ships_than_the_rival_delivered() {
    let mut sm = sim(with(base(), vec![sun(-1, 0.0, 0.0, 2, 0)]));
    for i in 0..6 {
        sm.ships.push(ship(1, 300.0 + i as f64, 0.0, Target::Obj(2)));
    }
    for i in 0..9 {
        sm.ships.push(ship(0, -300.0 - i as f64, 0.0, Target::Obj(2)));
    }
    sm.step(&[]);
    // 6 for faction 1 then 9 for faction 0 cancel then rebuild: faction 0 net 3, not captured
    assert_eq!(s(&sm, 2).faction, -1);
}

// ---------------------------------------------------------------- 3. orders
#[test]
fn order_sends_the_chosen_fraction_in_a_swarm_that_spreads() {
    let mut sm = sim(with(base(), vec![sun(-1, 0.0, 0.0, 2, 0)]));
    sm.issue(order(0, 0, Target::Obj(2), 50)).unwrap();
    assert_eq!(sm.ships.len(), 50);
    assert_eq!(s(&sm, 0).stock, 50);
    assert!(sm.ships.iter().all(|x| x.faction == 0));
    let xs: Vec<f64> = sm.ships.iter().map(|x| x.pos.x).collect();
    assert!(xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min) > 50.0, "ships spread out");
    run(&mut sm, 60);
    let heads: Vec<f64> = sm.ships.iter().map(|x| x.heading.x).collect();
    assert!(heads.iter().cloned().fold(f64::MIN, f64::max) - heads.iter().cloned().fold(f64::MAX, f64::min) > 0.0005, "headings differ slightly");
}

#[test]
fn swarm_flies_straight_towards_the_target() {
    let mut sm = sim(with(base(), vec![sun(-1, 0.0, 0.0, 2, 0)]));
    sm.issue(order(0, 0, Target::Obj(2), 100)).unwrap();
    let d0 = sm.ships[0].pos.dist(Vec2::ZERO);
    run(&mut sm, 30);
    let d1 = sm.ships.iter().map(|x| x.pos.dist(Vec2::ZERO)).fold(f64::MAX, f64::min);
    assert!(d1 < d0 - 300.0, "closer by {}", d0 - d1);
}

#[test]
fn order_to_empty_space_sends_ships_to_that_point_where_they_wait() {
    let mut sm = sim(base());
    let p = Vec2::new(-4000.0, 6000.0);
    sm.issue(order(0, 0, Target::Point(p), 30)).unwrap();
    run(&mut sm, 900);
    assert_eq!(sm.ships.len(), 30);
    assert!(sm.ships.iter().all(|x| x.target == Target::None && x.pos.dist(p) < POINT_ARRIVE + 1.0));
}

#[test]
fn orders_come_from_several_suns_at_once() {
    let mut sm = sim(vec![sun(0, -9000.0, 9000.0, 3, 100), sun(1, 9000.0, 9000.0, 3, 100), sun(0, -9000.0, 0.0, 3, 40)]);
    sm.issue(Command::Order { faction: 0, sources: vec![0, 2], target: Target::Point(Vec2::ZERO), then: None, percent: 50, repeat: false }).unwrap();
    assert_eq!(sm.ships.len(), 50 + 20);
}

#[test]
fn cannot_order_with_somebody_elses_sun_or_bad_targets() {
    let mut sm = sim(base());
    assert_eq!(sm.issue(order(0, 1, Target::Obj(0), 50)), Err(OrderError::NotYourSun));
    assert_eq!(sm.issue(order(0, 0, Target::Obj(99), 50)), Err(OrderError::BadTarget));
    assert_eq!(sm.issue(order(0, 0, Target::None, 50)), Err(OrderError::BadTarget));
    assert_eq!(sm.issue(order(0, 0, Target::Obj(1), 0)), Err(OrderError::BadPercent));
    assert_eq!(sm.issue(order(0, 5, Target::Obj(1), 50)), Err(OrderError::NotYourSun));
    assert_eq!(sm.issue(order(7, 0, Target::Obj(1), 50)), Err(OrderError::UnknownFaction));
}

#[test]
fn there_is_a_cap_on_simultaneous_orders() {
    let mut sm = sim(base());
    for _ in 0..8 {
        sm.issue(order(0, 0, Target::Obj(1), 1)).unwrap();
    }
    assert_eq!(sm.issue(order(0, 0, Target::Obj(1), 1)), Err(OrderError::TooManyOrders));
    run(&mut sm, 61);
    assert!(sm.issue(order(0, 0, Target::Obj(1), 1)).is_ok(), "the window rolls over");
}

#[test]
fn repeat_order_sends_new_production_until_cancelled() {
    let mut sm = sim(base());
    sm.issue(Command::Order { faction: 0, sources: vec![0], target: Target::Point(Vec2::new(0.0, 9000.0)), then: None, percent: 100, repeat: true }).unwrap();
    assert_eq!(s(&sm, 0).stock, 0);
    run(&mut sm, 600);
    assert_eq!(s(&sm, 0).stock, 0, "all production leaves");
    assert_eq!(sm.ships.len(), 100 + 10);
    sm.issue(Command::Cancel { faction: 0, sources: vec![0] }).unwrap();
    run(&mut sm, 600);
    assert_eq!(s(&sm, 0).stock, 10, "production stays home again");
}

#[test]
fn repeat_order_is_dropped_when_the_target_is_lost() {
    let frag = Object::FragileSun(FragileDef { sun: sdef(-1, 0.0, 0.0, 1, 0), orbit: OrbitDef::default(), broken_seconds: 600, rebuild_frames: 10 });
    let mut sm = sim(with(base(), vec![frag]));
    sm.issue(Command::Order { faction: 0, sources: vec![0], target: Target::Obj(2), then: None, percent: 1, repeat: true }).unwrap();
    // destroy the target by capturing it
    for i in 0..12 {
        sm.ships.push(ship(1, 235.0 + i as f64 * 0.1, 0.0, Target::Obj(2)));
    }
    sm.step(&[]);
    assert!(!s(&sm, 2).present());
    run(&mut sm, 70);
    assert!(s(&sm, 0).repeat.is_none());
}

// ---------------------------------------------------------------- 4. combat
fn survivors(sm: &Sim, f: u8) -> usize {
    sm.ships.iter().filter(|x| x.faction == f).count()
}

#[test]
fn equal_swarms_annihilate_and_the_larger_keeps_the_difference() {
    let mut sm = sim(base());
    for i in 0..10 {
        sm.ships.push(ship(0, i as f64, 0.0, Target::None));
    }
    for i in 0..7 {
        sm.ships.push(ship(1, i as f64, 5.0, Target::None));
    }
    sm.step(&[]);
    assert_eq!(survivors(&sm, 0), 3);
    assert_eq!(survivors(&sm, 1), 0);

    let mut eq = sim(base());
    for i in 0..8 {
        eq.ships.push(ship(0, i as f64, 0.0, Target::None));
        eq.ships.push(ship(1, i as f64, 5.0, Target::None));
    }
    eq.step(&[]);
    assert_eq!(eq.ships.len(), 0);
}

#[test]
fn friendly_ships_never_hurt_each_other() {
    let mut sm = sim(base());
    for i in 0..20 {
        sm.ships.push(ship(0, i as f64, 0.0, Target::None));
    }
    run(&mut sm, 5);
    assert_eq!(sm.ships.len(), 20);
}

#[test]
fn distant_enemies_do_not_fight() {
    let mut sm = sim(base());
    sm.ships.push(ship(0, 0.0, 0.0, Target::None));
    sm.ships.push(ship(1, COMBAT_RANGE * 3.0, 0.0, Target::None));
    run(&mut sm, 3);
    assert_eq!(sm.ships.len(), 2);
}

#[test]
fn combat_is_symmetric_regardless_of_faction_order() {
    for (a, b) in [(0u8, 1u8), (1, 0)] {
        let mut sm = sim(base());
        for i in 0..5 {
            sm.ships.push(ship(a, i as f64, 0.0, Target::None));
        }
        for i in 0..9 {
            sm.ships.push(ship(b, i as f64, 3.0, Target::None));
        }
        sm.step(&[]);
        assert_eq!((survivors(&sm, a), survivors(&sm, b)), (0, 4));
    }
}

// ---------------------------------------------------------------- 5. winning / losing
#[test]
fn faction_is_eliminated_with_no_suns_and_no_ships_and_human_wins() {
    let mut sm = sim(vec![sun(0, 0.0, 0.0, 3, 100), sun(1, 3000.0, 0.0, 1, 0)]);
    if let Obj::Sun(x) = &mut sm.objs[1] {
        x.level = 0;
    }
    sm.ships.push(ship(0, 3200.0, 0.0, Target::Obj(1)));
    assert_eq!(sm.status, Status::Playing);
    sm.step(&[]);
    assert_eq!(sm.status, Status::Won);
    assert!(sm.take_events().iter().any(|e| matches!(e, Event::Eliminated { faction: 1 })));
}

#[test]
fn faction_with_ships_but_no_suns_is_still_alive() {
    let mut sm = sim(vec![sun(0, 0.0, 0.0, 3, 100), sun(1, 3000.0, 0.0, 1, 0)]);
    if let Obj::Sun(x) = &mut sm.objs[1] {
        x.level = 0;
    }
    sm.ships.push(ship(0, 3200.0, 0.0, Target::Obj(1)));
    sm.ships.push(ship(1, -5000.0, 5000.0, Target::None));
    sm.step(&[]);
    assert_eq!(sm.status, Status::Playing);
    assert!(s(&sm, 1).faction == -1);
}

#[test]
fn human_loses_when_eliminated_and_game_freezes() {
    let mut sm = sim(vec![sun(0, 0.0, 0.0, 3, 0), sun(1, 3000.0, 0.0, 3, 100)]);
    if let Obj::Sun(x) = &mut sm.objs[0] {
        x.level = 0;
    }
    sm.ships.push(ship(1, 300.0, 0.0, Target::Obj(0)));
    sm.step(&[]);
    assert_eq!(sm.status, Status::Lost);
    let t = sm.tick;
    run(&mut sm, 10);
    assert_eq!(sm.tick, t, "a decided game stops stepping");
}

#[test]
fn surrender_loses_the_game() {
    let mut sm = sim(base());
    sm.step(&[Command::Surrender { faction: 0 }]);
    assert_eq!(sm.status, Status::Lost);
}

// ---------------------------------------------------------------- 6. special objects
fn orbiter(f: i32, cx: f64, cy: f64, rx: f64, ry: f64, speed: f64) -> Object {
    Object::OrbitingSun(sdef(f, cx, cy, 2, 0), OrbitDef { rx, ry, rotation_deg: 0.0, initial_angle_deg: 0.0, speed })
}

#[test]
fn orbiting_sun_travels_a_closed_ellipse_around_its_stored_position() {
    let mut sm = sim(with(base(), vec![orbiter(-1, 0.0, 0.0, 2000.0, 1000.0, 12.0)]));
    let p0 = s(&sm, 2).pos;
    assert!((p0.x - 2000.0).abs() < 1.0 && p0.y.abs() < 1.0);
    run(&mut sm, 15 * 60); // quarter turn at 12/2 = 6 deg/s -> 60 s per lap, so 15 s = 90 degrees
    let p1 = s(&sm, 2).pos;
    assert!(p1.x.abs() < 5.0 && (p1.y - 1000.0).abs() < 5.0, "{p1:?}");
    run(&mut sm, 45 * 60);
    let p2 = s(&sm, 2).pos;
    assert!(p2.dist(p0) < 5.0, "closed path: {p2:?}");
}

#[test]
fn orbit_rotation_tilts_the_ellipse() {
    let o = OrbitDef { rx: 1000.0, ry: 0.0001, rotation_deg: 90.0, initial_angle_deg: 0.0, speed: 0.0 };
    let p = orbit_pos(Vec2::ZERO, &o, 0);
    assert!(p.x.abs() < 1e-6 && (p.y - 1000.0).abs() < 1e-6, "{p:?}");
}

#[test]
fn ships_track_an_orbiting_sun() {
    let mut sm = sim(with(base(), vec![orbiter(-1, 0.0, 0.0, 2500.0, 2500.0, 18.0)]));
    sm.issue(order(0, 0, Target::Obj(2), 15)).unwrap();
    for _ in 0..3000 {
        sm.step(&[]);
        if s(&sm, 2).faction == 0 {
            return;
        }
    }
    panic!("swarm never caught the orbiting sun");
}

fn fragile(x: f64, y: f64, broken: u32) -> Object {
    Object::FragileSun(FragileDef { sun: sdef(-1, x, y, 2, 0), orbit: OrbitDef::default(), broken_seconds: broken, rebuild_frames: 60 })
}

#[test]
fn fragile_sun_is_destroyed_when_taken_then_rebuilds_as_an_empty_neutral_sun() {
    let mut sm = sim(with(base(), vec![fragile(0.0, 0.0, 12)]));
    for i in 0..12 {
        sm.ships.push(ship(0, 300.0 + i as f64, 0.0, Target::Obj(2)));
    }
    sm.step(&[]);
    assert!(!s(&sm, 2).present(), "taking a fragile sun destroys it");
    assert_eq!(s(&sm, 2).faction, -1);
    assert!(sm.take_events().iter().any(|e| matches!(e, Event::SunDestroyed { sun: 2 })));
    // gone for brokenseconds * 10 ticks = 120, then 60 rebuild ticks
    run(&mut sm, 100);
    assert!(!s(&sm, 2).present());
    run(&mut sm, 100);
    assert!(s(&sm, 2).present());
    assert_eq!((s(&sm, 2).faction, s(&sm, 2).level, s(&sm, 2).stock), (-1, 0, 0));
}

#[test]
fn ships_pass_through_a_destroyed_fragile_sun() {
    let mut sm = sim(with(base(), vec![fragile(0.0, 0.0, 600)]));
    for i in 0..10 {
        sm.ships.push(ship(0, 250.0 + i as f64 * 0.1, 0.0, Target::Obj(2)));
    }
    sm.step(&[]);
    assert!(!s(&sm, 2).present());
    sm.ships.push(ship(1, -1000.0, 0.0, Target::Point(Vec2::new(1000.0, 0.0))));
    for i in 0..400 {
        sm.step(&[]);
        if sm.ships.iter().all(|x| x.faction != 1) {
            panic!("ship vanished at step {i}; status {:?}, ships {}", sm.status, sm.ships.len());
        }
    }
    let sh = sm.ships.iter().find(|x| x.faction == 1).expect("still flying");
    assert!(sh.pos.x > 900.0, "passed through the gap: {:?}", sh.pos);
}

#[test]
fn fragile_sun_destroyed_when_attacked_down_to_zero() {
    let mut sm = sim(base());
    // a fragile sun owned by faction 1 at level 0
    sm.objs.push(Obj::Sun(au2_sim::Sun {
        pos: Vec2::new(0.0, 0.0),
        centre: Vec2::ZERO,
        orbit: None,
        faction: 1,
        level: 0,
        max_level: 2,
        progress: 0,
        stock: 0,
        cap_faction: -1,
        cap_progress: 0,
        accum: 0,
        repeat: None,
        fragile: Some(Fragile { phase: FragilePhase::Intact, gone_ticks: 100, rebuild_ticks: 10 }),
        radius: 300.0,
    }));
    sm.ships.push(ship(0, 300.0, 0.0, Target::Obj(2)));
    sm.step(&[]);
    assert!(!s(&sm, 2).present());
}

fn wh(x: f64, y: f64, dest: i32) -> Object {
    Object::Wormhole(WormholeDef { destination: dest, size: 400.0, orbit: OrbitDef::default(), pos: [x, y] })
}

#[test]
fn wormhole_moves_a_ship_to_its_partner_keeping_heading() {
    let mut sm = sim(with(base(), vec![wh(0.0, 0.0, 3), wh(5000.0, 3000.0, 2)]));
    let mut sh = ship(0, -300.0, 0.0, Target::Point(Vec2::new(30000.0, 0.0)));
    sh.heading = Vec2::new(1.0, 0.0);
    sm.ships.push(sh);
    run(&mut sm, 60);
    let p = sm.ships[0].pos;
    assert!(p.x > 5000.0 && (p.y - 3000.0).abs() < 100.0, "{p:?}");
    assert!(sm.ships[0].heading.x > 0.9, "heading kept");
}

#[test]
fn wormhole_does_not_bounce_ships_back_and_forth() {
    let mut sm = sim(with(base(), vec![wh(0.0, 0.0, 3), wh(5000.0, 3000.0, 2)]));
    sm.ships.push(ship(0, -300.0, 0.0, Target::Point(Vec2::new(30000.0, 3000.0))));
    run(&mut sm, 600);
    assert!(sm.ships[0].pos.x > 6000.0, "{:?}", sm.ships[0].pos);
}

#[test]
fn wormhole_with_random_destination_picks_another_wormhole() {
    let mut sm = sim(with(base(), vec![wh(0.0, 0.0, -1), wh(5000.0, 3000.0, -1), wh(-5000.0, -3000.0, -1)]));
    sm.ships.push(ship(0, -300.0, 0.0, Target::Point(Vec2::new(9000.0, 0.0))));
    run(&mut sm, 3);
    let p = sm.ships[0].pos;
    assert!(p.dist(Vec2::ZERO) > 3000.0, "{p:?}");
}

#[test]
fn wormhole_can_be_an_order_target_and_continues_to_the_follow_up() {
    let mut sm = sim(with(base(), vec![wh(0.0, 0.0, 3), wh(5000.0, 3000.0, 2), sun(-1, 6500.0, 3000.0, 1, 0)]));
    sm.issue(Command::Order { faction: 0, sources: vec![0], target: Target::Obj(2), then: Some(Target::Obj(4)), percent: 15, repeat: false }).unwrap();
    for _ in 0..3000 {
        sm.step(&[]);
        if s(&sm, 4).faction == 0 {
            return;
        }
    }
    panic!("never captured via wormhole");
}

fn invest_objs(kind: &str) -> Vec<Object> {
    let o = match kind {
        "white" => Object::WhiteHole(WhiteHoleDef { required_investment: 5, start_return: 40, end_return: 60, growth_rate: 0.0, game_size: 300.0, dormant_period: 120, pos: [0.0, 0.0] }),
        _ => Object::Supernova(SupernovaDef {
            required_investment: 5,
            start_scale: 400.0,
            end_scale: 600.0,
            blast_radius: 1500.0,
            collapse_radius: 2000.0,
            kill_enemy_suns: false,
            kill_enemy_units: true,
            unit_residuals: 11,
            dormant_period: 120,
            pos: [0.0, 0.0],
        }),
    };
    with(base(), vec![o])
}

#[test]
fn white_hole_releases_a_burst_for_the_investor_then_goes_dormant() {
    let mut sm = sim(invest_objs("white"));
    for i in 0..5 {
        sm.ships.push(ship(0, 100.0 + i as f64, 0.0, Target::Obj(2)));
    }
    sm.step(&[]);
    let n = survivors(&sm, 0);
    assert!((40..=60).contains(&n), "burst of {n}");
    assert!(sm.take_events().iter().any(|e| matches!(e, Event::WhiteHole { faction: 0, .. })));
    // dormant: further investment is ignored
    let before = sm.ships.len();
    for i in 0..5 {
        sm.ships.push(ship(0, 100.0 + i as f64, 10.0, Target::Obj(2)));
    }
    sm.step(&[]);
    assert!(sm.ships.len() <= before + 5 - 4, "no second burst while dormant");
}

#[test]
fn investment_is_tracked_per_faction() {
    let mut sm = sim(invest_objs("white"));
    for i in 0..3 {
        sm.ships.push(ship(0, 100.0 + i as f64, 0.0, Target::Obj(2)));
        sm.ships.push(ship(1, 100.0 + i as f64, 2000.0, Target::Obj(2)));
    }
    // faction 1's ships are out of arrival range, so only 3 invested: nothing released
    sm.step(&[]);
    assert_eq!(sm.ships.len(), 3);
    if let Obj::WhiteHole(w) = &sm.objs[2] {
        assert_eq!(w.invest[0], 3);
        assert_eq!(w.invest[1], 0);
    }
}

#[test]
fn supernova_destroys_enemy_ships_in_the_blast_radius_only() {
    let mut sm = sim(invest_objs("nova"));
    for i in 0..5 {
        sm.ships.push(ship(0, 100.0 + i as f64, 0.0, Target::Obj(2)));
    }
    sm.ships.push(ship(1, 1000.0, 0.0, Target::None)); // inside the blast
    sm.ships.push(ship(1, 4000.0, 0.0, Target::None)); // outside
    sm.ships.push(ship(0, 900.0, 600.0, Target::None)); // friendly inside
    sm.step(&[]);
    assert_eq!(survivors(&sm, 1), 1);
    assert!(sm.ships.iter().any(|x| x.faction == 1 && x.pos.x > 3000.0));
    assert!(survivors(&sm, 0) >= 1 + 11, "residual ships for the investor: {}", survivors(&sm, 0));
}

#[test]
fn supernova_can_kill_enemy_suns_when_the_level_says_so() {
    let mut objs = invest_objs("nova");
    if let Object::Supernova(d) = &mut objs[2] {
        d.kill_enemy_suns = true;
    }
    objs.push(sun(1, 800.0, 0.0, 2, 50));
    let mut sm = Sim::new(&map(2, objs), SimConfig::default());
    for i in 0..5 {
        sm.ships.push(ship(0, 100.0 + i as f64, 0.0, Target::Obj(2)));
    }
    sm.step(&[]);
    assert_eq!(s(&sm, 3).faction, -1);
    assert_eq!(s(&sm, 1).faction, 1, "far-away sun survives");
}

#[test]
fn gamma_ray_sweeps_and_destroys_suns_it_touches() {
    let g = Object::GammaRay(GammaRayDef { kill_suns: true, kill_units: false, rotation_deg: 0.0, width: 800.0, period: 60, randomization: 0.0, random_start: false, pos: [0.0, 0.0] });
    let mut sm = sim(with(base(), vec![g, sun(1, 3000.0, 0.0, 2, 10), sun(1, -3000.0, 0.0, 2, 10)]));
    // the beam also sweeps both home suns; keep each faction alive with a ship parked at the harmless hub
    sm.ships.push(ship(0, 0.0, 100.0, Target::None));
    sm.ships.push(ship(1, 0.0, -100.0, Target::None));
    run(&mut sm, 2);
    assert_eq!(s(&sm, 3).faction, -1, "sun on the beam is lost");
    assert_eq!(s(&sm, 4).faction, 1, "sun on the far side is not yet touched");
    // a full revolution takes period * GAMMA_TICKS_PER_PERIOD ticks; the beam reaches the opposite side half-way
    run(&mut sm, 60 * GAMMA_TICKS_PER_PERIOD / 2);
    assert_eq!(s(&sm, 4).faction, -1, "the sweep reached the other sun");
}

#[test]
fn gamma_ray_destroys_ships_only_when_configured() {
    for (kill, left) in [(true, 0usize), (false, 1)] {
        let g = Object::GammaRay(GammaRayDef { kill_suns: false, kill_units: kill, rotation_deg: 0.0, width: 800.0, period: 60, randomization: 0.0, random_start: false, pos: [0.0, 0.0] });
        let mut sm = sim(with(base(), vec![g]));
        sm.ships.push(ship(0, 3000.0, 0.0, Target::None));
        run(&mut sm, 2);
        assert_eq!(sm.ships.len(), left);
    }
}

#[test]
fn gamma_ray_random_start_depends_on_seed_but_is_reproducible() {
    let g = || Object::GammaRay(GammaRayDef { kill_suns: true, kill_units: false, rotation_deg: 0.0, width: 800.0, period: 60, randomization: 0.0, random_start: true, pos: [0.0, 0.0] });
    let angle = |seed| {
        let sm = Sim::new(&map(2, with(base(), vec![g()])), SimConfig { seed, ..Default::default() });
        if let Obj::Gamma(g) = &sm.objs[2] { g.angle } else { panic!() }
    };
    assert_eq!(angle(5), angle(5));
    assert_ne!(angle(5), angle(6));
}

#[test]
fn shockwave_ring_destroys_ships_it_touches() {
    let sw = Object::Shockwave(ShockwaveDef { kind: "st_incircle".into(), dormant: 0, period: 600, speed: 10.0, lifetime: 500, line_rotation_deg: 0.0, start_offset: 0.0, pos: [0.0, 0.0] });
    let mut sm = sim(with(base(), vec![sw]));
    sm.ships.push(ship(0, 1000.0, 0.0, Target::None)); // ring reaches r=1000 after ~100 ticks
    sm.ships.push(ship(0, 12000.0, 0.0, Target::None)); // never reached (lifetime 500 * 10 = 5000)
    run(&mut sm, 80);
    assert_eq!(sm.ships.len(), 2);
    run(&mut sm, 40);
    assert_eq!(sm.ships.len(), 1);
}

#[test]
fn fog_of_war_limits_what_a_faction_sees() {
    let fog = Object::Fog(FogDef { sun_radius: 2000.0, additional_per_ring: 0.0, unowned_radius: 0.0, enemy_radius: 0.0, non_sun_radius: 500.0, pos: [0.0, 0.0] });
    let mut sm = sim(with(base(), vec![fog]));
    let v = sm.vision(0);
    assert!(v.visible(Vec2::new(-9000.0, 9000.0)));
    assert!(v.visible(Vec2::new(-8000.0, 9000.0)));
    assert!(!v.visible(Vec2::new(9000.0, 9000.0)), "enemy home hidden");
    sm.ships.push(ship(0, 0.0, 0.0, Target::None));
    assert!(sm.vision(0).visible(Vec2::new(400.0, 0.0)));
    assert!(!sm.vision(0).visible(Vec2::new(600.0, 0.0)));
    // without fog everything is visible
    assert!(sim(base()).vision(0).visible(Vec2::new(9000.0, 9000.0)));
}

// ---------------------------------------------------------------- 7. terrain
fn terr(t: TerrainDef) -> Object {
    Object::Terrain(t)
}

#[test]
fn force_terrain_pushes_ships() {
    let mut t = terrain(TerrainKind::Force, -1000.0, -1000.0, 2, 2, 1000.0);
    t.acceleration = [30.0, 0.0];
    let mut sm = sim(with(base(), vec![terr(t)]));
    sm.ships.push(ship(0, 0.0, 0.0, Target::None));
    run(&mut sm, 30);
    assert!(sm.ships[0].pos.x > 100.0, "{:?}", sm.ships[0].pos);
    assert!(sm.ships[0].pos.y.abs() < 1.0);
}

#[test]
fn speed_terrain_changes_ship_speed() {
    let travel = |change: f64| {
        let mut t = terrain(TerrainKind::Speed, -1000.0, -1000.0, 2, 30, 1000.0);
        t.speed_change = change;
        let mut sm = sim(with(base(), vec![terr(t)]));
        sm.ships.push(ship(0, 0.0, 0.0, Target::Point(Vec2::new(20000.0, 0.0))));
        run(&mut sm, 30);
        sm.ships[0].pos.x
    };
    let normal = travel(0.0);
    assert!(travel(32.0) > normal * 1.8);
    assert!(travel(-16.0) < normal * 0.6);
}

#[test]
fn reflect_terrain_is_a_wall_ships_bounce_off() {
    let t = terrain(TerrainKind::Reflect, 1000.0, -2000.0, 8, 1, 500.0);
    let mut sm = sim(with(base(), vec![terr(t)]));
    sm.ships.push(ship(0, 0.0, 0.0, Target::Point(Vec2::new(5000.0, 0.0))));
    let mut bounced = false;
    for _ in 0..300 {
        sm.step(&[]);
        let sh = &sm.ships[0];
        assert!(sh.pos.x < 1000.0, "inside the wall: {:?}", sh.pos);
        bounced |= sh.heading.x < 0.0;
    }
    assert!(bounced);
}

#[test]
fn minefield_kills_ships_at_a_steady_rate_inside_only() {
    let mut t = terrain(TerrainKind::Minefield, -1000.0, -1000.0, 4, 4, 500.0);
    t.death_rate = 50.0;
    t.density = 100.0;
    let mut sm = sim(with(base(), vec![terr(t)]));
    for i in 0..200 {
        sm.ships.push(ship(0, -900.0 + (i % 10) as f64 * 150.0, -900.0 + (i / 10) as f64 * 90.0, Target::None));
    }
    for i in 0..50 {
        sm.ships.push(ship(0, 5000.0 + i as f64, 5000.0, Target::None));
    }
    run(&mut sm, 60);
    let inside_left = sm.ships.iter().filter(|x| x.pos.x < 2000.0).count();
    assert!(inside_left < 200 && inside_left > 20, "partial attrition after 1 s: {inside_left}");
    run(&mut sm, 1200);
    assert_eq!(sm.ships.iter().filter(|x| x.pos.x < 2000.0).count(), 0);
    assert_eq!(sm.ships.iter().filter(|x| x.pos.x > 2000.0).count(), 50);
}

#[test]
fn neutral_zone_ships_fight_nobody() {
    let t = terrain(TerrainKind::Neutral, -1000.0, -1000.0, 4, 4, 500.0);
    let mut sm = sim(with(base(), vec![terr(t)]));
    for i in 0..5 {
        sm.ships.push(ship(0, i as f64, 0.0, Target::None));
        sm.ships.push(ship(1, i as f64, 4.0, Target::None));
    }
    run(&mut sm, 10);
    assert_eq!(sm.ships.len(), 10, "no fighting inside");
    // once out, they fight
    for sh in sm.ships.iter_mut() {
        sh.pos.x += 3000.0;
    }
    run(&mut sm, 2);
    assert_eq!(sm.ships.len(), 0);
}

#[test]
fn confusion_zone_flips_factions_or_drops_orders() {
    let mut t = terrain(TerrainKind::Confusion, -1000.0, -1000.0, 4, 4, 500.0);
    t.faction_change_rate = 100.0;
    let mut sm = sim(with(base(), vec![terr(t)]));
    for i in 0..30 {
        sm.ships.push(ship(0, -900.0 + i as f64 * 5.0, 0.0, Target::None));
    }
    run(&mut sm, 20);
    // some changed faction (and then fought their former friends)
    assert!(sm.ships.iter().any(|x| x.faction == 1) || sm.ships.len() < 30);

    let mut t2 = terrain(TerrainKind::Confusion, -1000.0, -1000.0, 4, 4, 500.0);
    t2.lose_order_rate = 100.0;
    let mut sm = sim(with(base(), vec![terr(t2)]));
    sm.ships.push(ship(0, 0.0, 0.0, Target::Point(Vec2::new(8000.0, 0.0))));
    run(&mut sm, 120);
    assert_eq!(sm.ships[0].target, Target::None);
}

#[test]
fn circular_zone_is_an_annulus() {
    let mut t = terrain(TerrainKind::Minefield, 0.0, 0.0, 0, 0, 100.0);
    t.data.clear();
    t.outer_radius = 1000.0;
    t.inner_radius = 500.0;
    let tt = Terrain { def: t, enabled: true, accel: Vec2::ZERO };
    assert!(!tt.contains(Vec2::new(100.0, 0.0)));
    assert!(tt.contains(Vec2::new(700.0, 0.0)));
    assert!(!tt.contains(Vec2::new(1100.0, 0.0)));
}

#[test]
fn switching_terrain_toggles() {
    let mut t = terrain(TerrainKind::Minefield, -1000.0, -1000.0, 4, 4, 500.0);
    t.switch_time = 100;
    t.death_rate = 100.0;
    let mut sm = sim(with(base(), vec![terr(t)]));
    run(&mut sm, 100);
    if let Obj::Terrain(x) = &sm.objs[2] {
        assert!(!x.enabled);
    }
    run(&mut sm, 100);
    if let Obj::Terrain(x) = &sm.objs[2] {
        assert!(x.enabled);
    }
}

// ---------------------------------------------------------------- 9. time & determinism
fn scripted(seed: u64, swap: bool) -> u64 {
    let objs = with(base(), vec![sun(-1, 0.0, 0.0, 2, 0), sun(-1, 3000.0, 3000.0, 1, 0)]);
    let mut sm = Sim::new(&map(2, objs), SimConfig { seed, ai_difficulty: 2, human_ai: false, enable_ai: true });
    for t in 0..300u32 {
        let mut cmds = Vec::new();
        if t == 5 {
            cmds.push(order(0, 0, Target::Obj(2), 40));
        }
        if t == (if swap { 7 } else { 6 }) {
            cmds.push(order(0, 0, Target::Obj(3), 30));
        }
        sm.step(&cmds);
    }
    sm.state_hash()
}

#[test]
fn same_level_seed_and_orders_give_identical_results() {
    assert_eq!(scripted(3, false), scripted(3, false));
}

#[test]
fn different_order_timing_or_seed_changes_the_outcome() {
    assert_ne!(scripted(3, false), scripted(3, true));
    assert_ne!(scripted(3, false), scripted(4, false));
}

#[test]
fn step_rate_is_sixty_per_second() {
    assert_eq!(TICKS_PER_SEC, 60);
    let mut sm = sim(base());
    run(&mut sm, 120);
    assert_eq!(sm.tick, 120);
}

#[test]
fn deterministic_trig_matches_reality_closely() {
    use au2_sim::math::*;
    for d in (-720..=720).step_by(7) {
        let r = (d as f64).to_radians();
        assert!((sin_deg(d as f64) - r.sin()).abs() < 1e-9, "sin {d}");
        assert!((cos_deg(d as f64) - r.cos()).abs() < 1e-9, "cos {d}");
    }
}

#[test]
fn rng_is_seeded_and_repeatable() {
    use au2_sim::math::Rng;
    let a: Vec<u32> = { let mut r = Rng::new(9); (0..5).map(|_| r.next_u32()).collect() };
    let b: Vec<u32> = { let mut r = Rng::new(9); (0..5).map(|_| r.next_u32()).collect() };
    let c: Vec<u32> = { let mut r = Rng::new(10); (0..5).map(|_| r.next_u32()).collect() };
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// ---------------------------------------------------------------- 8. AI
#[test]
fn ai_expands_to_neutral_suns_and_upgrades() {
    let objs = with(
        vec![sun(0, -4000.0, 0.0, 3, 100), sun(1, 4000.0, 0.0, 3, 100)],
        vec![sun(-1, -1500.0, 0.0, 1, 0), sun(-1, 1500.0, 0.0, 1, 0)],
    );
    let mut sm = Sim::new(&map(2, objs), SimConfig { seed: 1, ai_difficulty: 2, human_ai: false, enable_ai: true });
    run(&mut sm, 60 * 90);
    assert_eq!(s(&sm, 3).faction, 1, "AI faction took the nearer neutral sun");
    assert!(s(&sm, 1).level >= 2 || sm.sun_count(1) >= 2);
}
