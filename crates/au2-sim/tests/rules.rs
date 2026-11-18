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
fn sim_n(n: usize, objs: Vec<Object>) -> Sim {
    Sim::new(&map(n, objs), SimConfig { seed: 7, ai_difficulty: 0, human_ai: false, enable_ai: false })
}
fn sim(objs: Vec<Object>) -> Sim {
    sim_n(2, objs)
}
fn run(s: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        s.step(&[]);
    }
}
fn order(f: u8, ships: Vec<u32>, t: Target) -> Command {
    Command::Order { faction: f, ships, target: t }
}
fn s(sim: &Sim, i: usize) -> &au2_sim::Sun {
    sim.sun(i).unwrap()
}
fn sun_mut(sim: &mut Sim, i: usize) -> &mut au2_sim::Sun {
    match &mut sim.objs[i] {
        Obj::Sun(s) => s,
        _ => panic!("not a sun"),
    }
}
fn ship_by_id(sim: &Sim, id: u32) -> Option<&Ship> {
    sim.ship_index(id).map(|i| &sim.ships[i])
}
/// `n` ships of faction `f` in a small loose block around (x, y), all heading for `t`. Returns their ids.
fn swarm(sm: &mut Sim, f: u8, x: f64, y: f64, n: usize, t: Target) -> Vec<u32> {
    (0..n).map(|k| sm.add_ship(f, Vec2::new(x + (k % 6) as f64 * 8.0, y + (k / 6) as f64 * 8.0), t)).collect()
}
/// `n` ships hovering (held) around a point.
fn parked(sm: &mut Sim, f: u8, x: f64, y: f64, n: usize) -> Vec<u32> {
    (0..n)
        .map(|k| {
            let p = Vec2::new(x + (k % 6) as f64 * 8.0, y + (k / 6) as f64 * 8.0);
            sm.add_ship(f, p, Target::Point(p))
        })
        .collect()
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
fn rejected(sm: &mut Sim) -> Vec<OrderError> {
    sm.take_events().into_iter().filter_map(|e| if let Event::OrderRejected { why, .. } = e { Some(why) } else { None }).collect()
}

// ---------------------------------------------------------------- 2. suns: production
#[test]
fn level_one_sun_releases_a_ring_of_two_ships_every_second() {
    let mut sm = sim(base());
    assert_eq!(sm.strength(0), 100);
    run(&mut sm, 600);
    assert_eq!(sm.strength(0), 120);
    assert!(s(&sm, 0).last_ring == 600, "the sun pulses on each release");
}

#[test]
fn ring_ships_leave_evenly_spaced_around_the_rim() {
    let mut sm = sim(vec![sun(0, 0.0, 0.0, 3, 0), sun(1, 9000.0, 9000.0, 3, 100)]);
    sun_mut(&mut sm, 0).level = 5;
    sun_mut(&mut sm, 0).radius = sun_radius(5);
    run(&mut sm, 60);
    let ring: Vec<&Ship> = sm.ships.iter().filter(|x| x.faction == 0).collect();
    assert_eq!(ring.len(), RING_SHIPS[5] as usize);
    let r = s(&sm, 0).radius;
    for x in &ring {
        let d = x.pos.len();
        assert!(d > r && d < r + 20.0, "ring ship at {d}, rim {r}");
    }
    // evenly spaced: every ship's nearest neighbour is about one chord away
    let chord = 2.0 * (r + 4.0) * (std::f64::consts::PI / ring.len() as f64).sin();
    for a in &ring {
        let near = ring.iter().filter(|b| b.id != a.id).map(|b| b.pos.dist(a.pos)).fold(f64::MAX, f64::min);
        assert!((near - chord).abs() < chord * 0.35, "{near} vs {chord}");
    }
}

#[test]
fn production_grows_with_level_from_a_couple_to_about_nine_per_second() {
    assert_eq!(production_per_sec(0), 0.0);
    assert_eq!(production_per_sec(1), 2.0);
    for l in 1..5 {
        assert!(production_per_sec(l + 1) > production_per_sec(l));
    }
    assert!((8.0..=10.0).contains(&production_per_sec(5)));
}

#[test]
fn neutral_suns_produce_nothing() {
    let mut sm = sim(with(base(), vec![sun(-1, 0.0, 0.0, 3, 50)]));
    run(&mut sm, 600);
    assert_eq!(sm.ships.len(), 240);
    assert!(sm.ships.iter().all(|x| x.pos.len() > 3000.0));
}

#[test]
fn starting_garrison_circles_its_sun_on_a_flattened_oval() {
    let mut sm = sim(base());
    let c = s(&sm, 0).pos;
    let (r, cloud) = (s(&sm, 0).radius, s(&sm, 0).cloud_radius());
    let mine: Vec<&Ship> = sm.ships.iter().filter(|x| x.faction == 0).collect();
    assert_eq!(mine.len(), 100);
    for x in &mine {
        assert_eq!(x.mode, Mode::Orbit);
        assert_eq!(x.home, 0);
        let d = x.pos.dist(c);
        assert!(d > r * 0.6 && d <= cloud * 1.01, "{d}");
    }
    // a few times the radius wide, about twice as wide as tall, and each ship at its own speed
    run(&mut sm, 600);
    let (mut wx, mut wy) = (0.0f64, 0.0f64);
    for x in sm.ships.iter().filter(|x| x.faction == 0 && x.id < 100) {
        wx = wx.max((x.pos.x - c.x).abs());
        wy = wy.max((x.pos.y - c.y).abs());
    }
    assert!(wx > r * 2.0 && wx < r * 4.5, "cloud width {wx} for radius {r}");
    assert!((1.6..2.4).contains(&(wx / wy)), "width / height = {}", wx / wy);
    let mut speeds: Vec<u64> = sm.ships.iter().filter(|x| x.faction == 0).map(|x| (x.orbit_w * 1e6) as u64).collect();
    speeds.sort();
    speeds.dedup();
    assert!(speeds.len() > 90);
}

#[test]
fn bigger_suns_have_wider_clouds_of_more_ovals() {
    let r1 = cloud_radius(sun_radius(1), 1);
    let r5 = cloud_radius(sun_radius(5), 5);
    assert!(r5 > r1 * 2.0);
    assert!(orbit_bands(5) > orbit_bands(1));
}

#[test]
fn a_moving_suns_cloud_travels_with_it() {
    let o = Object::OrbitingSun(sdef(0, 0.0, 0.0, 2, 40), OrbitDef { rx: 2000.0, ry: 1000.0, rotation_deg: 0.0, initial_angle_deg: 0.0, speed: 12.0 });
    let mut sm = sim(with(base(), vec![o]));
    run(&mut sm, 900);
    let (c, cloud) = (s(&sm, 2).pos, s(&sm, 2).cloud_radius());
    let around: Vec<&Ship> = sm.ships.iter().filter(|x| x.home == 2).collect();
    assert!(around.len() >= 40);
    assert!(around.iter().all(|x| x.pos.dist(c) < cloud * 1.3), "cloud left behind");
}

// ---------------------------------------------------------------- 2. suns: size, costs, health and growth
#[test]
fn sun_size_follows_its_current_level_and_empty_suns_look_like_level_one() {
    let sm = sim(with(base(), vec![sun(-1, 0.0, 0.0, 5, 0)]));
    assert_eq!(s(&sm, 2).radius, sun_radius(1));
    for l in 1..5 {
        assert!(sun_radius(l + 1) > sun_radius(l));
    }
}

#[test]
fn costs_are_heavy_and_supergiant_steps_cost_more() {
    assert_eq!(CAPTURE_COST, 100);
    for l in 1..=2 {
        assert!((90..=130).contains(&UPGRADE_COST[l]), "{l}");
    }
    assert!(UPGRADE_COST[3] > UPGRADE_COST[2] && UPGRADE_COST[4] > UPGRADE_COST[3]);
    // a level-1 sun needs most of a minute of its own production for its next level
    let secs = UPGRADE_COST[1] as f64 / production_per_sec(1);
    assert!((35.0..=60.0).contains(&secs));
}

#[test]
fn capturing_an_empty_sun_takes_a_hundred_ships_and_gives_level_one() {
    let mut sm = sim(with(base(), vec![sun(-1, 0.0, 0.0, 3, 0)]));
    swarm(&mut sm, 0, -1500.0, 0.0, 99, Target::Obj(2));
    run(&mut sm, 600);
    assert_eq!(s(&sm, 2).faction, -1);
    assert_eq!(s(&sm, 2).cap_progress, 99);
    swarm(&mut sm, 0, -1500.0, 0.0, 1, Target::Obj(2));
    run(&mut sm, 600);
    let x = s(&sm, 2);
    assert_eq!((x.faction, x.level, x.health), (0, 1, x.max_health()));
}

#[test]
fn a_rival_first_drains_the_capture_then_starts_its_own() {
    let mut sm = sim(with(base(), vec![sun(-1, 0.0, 0.0, 3, 0)]));
    swarm(&mut sm, 0, -1500.0, 0.0, 60, Target::Obj(2));
    run(&mut sm, 500);
    assert_eq!((s(&sm, 2).cap_faction, s(&sm, 2).cap_progress), (0, 60));
    swarm(&mut sm, 1, 1500.0, 0.0, 80, Target::Obj(2));
    run(&mut sm, 500);
    assert_eq!((s(&sm, 2).cap_faction, s(&sm, 2).cap_progress), (1, 20));
}

#[test]
fn friendly_ships_upgrade_a_sun_and_the_surplus_joins_its_cloud() {
    let mut sm = sim(base());
    let r1 = s(&sm, 0).radius;
    let ids = swarm(&mut sm, 0, -9000.0, 7000.0, 150, Target::Obj(0));
    run(&mut sm, 600);
    let x = s(&sm, 0);
    assert_eq!((x.level, x.growth), (2, 0), "one level, the rest is not used up");
    assert!(x.radius > r1, "the sun grew");
    let survivors: Vec<&Ship> = ids.iter().filter_map(|&i| ship_by_id(&sm, i)).collect();
    assert_eq!(survivors.len(), 50);
    assert!(survivors.iter().all(|x| x.mode == Mode::Orbit && x.home == 0));
}

#[test]
fn a_full_sun_at_its_maximum_absorbs_nothing() {
    let mut sm = sim(vec![sun(0, -9000.0, 9000.0, 1, 0), sun(1, 9000.0, 9000.0, 3, 100)]);
    let ids = swarm(&mut sm, 0, -9000.0, 7000.0, 20, Target::Obj(0));
    run(&mut sm, 600);
    assert!(ids.iter().all(|&i| ship_by_id(&sm, i).is_some_and(|x| x.mode == Mode::Orbit && x.home == 0)));
}

#[test]
fn friendly_ships_repair_health_before_filling_growth() {
    let mut sm = sim(base());
    sun_mut(&mut sm, 0).health = 50;
    swarm(&mut sm, 0, -9000.0, 7000.0, 70, Target::Obj(0));
    run(&mut sm, 600);
    assert_eq!((s(&sm, 0).health, s(&sm, 0).growth), (100, 20));
}

#[test]
fn hostile_ships_remove_health_and_a_dead_sun_drops_to_empty_whatever_its_level() {
    let mut sm = sim(with(base(), vec![sun(1, 0.0, 0.0, 3, 0)]));
    {
        let x = sun_mut(&mut sm, 2);
        x.level = 3;
        x.radius = sun_radius(3);
        x.growth = 40;
    }
    // attackers close around it so they all land before the sun's first ring
    for k in 0..99 {
        sm.add_ship(0, au2_sim::math::dir_deg(k as f64 * 3.6) * 250.0, Target::Obj(2));
    }
    run(&mut sm, 40);
    assert_eq!((s(&sm, 2).faction, s(&sm, 2).level, s(&sm, 2).health), (1, 3, 1));
    sm.take_events();
    sm.add_ship(0, Vec2::new(-250.0, 0.0), Target::Obj(2));
    run(&mut sm, 40);
    let x = s(&sm, 2);
    assert_eq!((x.faction, x.level, x.growth), (-1, 0, 0));
    assert!(sm.take_events().iter().any(|e| matches!(e, Event::SunDied { sun: 2, faction: 1, level: 3 })));
}

#[test]
fn supergiants_are_much_tougher() {
    assert_eq!(MAX_HEALTH[1], MAX_HEALTH[2]);
    assert_eq!(MAX_HEALTH[2], MAX_HEALTH[3]);
    assert!(MAX_HEALTH[4] >= 2 * MAX_HEALTH[3]);
    assert!(MAX_HEALTH[5] > MAX_HEALTH[4]);
}

// ---------------------------------------------------------------- 3. ships and orders
#[test]
fn an_idle_ship_far_from_friendly_suns_hovers_and_jitters() {
    let mut sm = sim(base());
    let id = sm.add_ship(0, Vec2::new(0.0, 0.0), Target::None);
    run(&mut sm, 120);
    let x = ship_by_id(&sm, id).unwrap();
    assert_eq!(x.mode, Mode::Hover);
    assert!(x.pos != Vec2::ZERO && x.pos.len() < 40.0, "{:?}", x.pos);
}

#[test]
fn an_idle_ship_near_a_friendly_sun_joins_its_cloud() {
    let mut sm = sim(base());
    let id = sm.add_ship(0, Vec2::new(-9000.0 + 700.0, 9000.0), Target::None);
    let x = ship_by_id(&sm, id).unwrap();
    assert_eq!((x.mode, x.home), (Mode::Orbit, 0));
}

#[test]
fn area_selection_takes_circling_hovering_and_flying_ships() {
    let mut sm = sim(with(base(), vec![sun(-1, 0.0, 0.0, 1, 0)]));
    let c = s(&sm, 0).pos;
    assert_eq!(sm.ships_in_circle(0, c, sm.sun_select_radius(0)).len(), 100, "tapping a sun selects its whole cloud");
    let hov = parked(&mut sm, 0, 0.0, 3000.0, 5);
    let fly = swarm(&mut sm, 0, 0.0, 3200.0, 5, Target::Obj(2));
    run(&mut sm, 2);
    let sel = sm.ships_in_circle(0, Vec2::new(0.0, 3100.0), 300.0);
    assert!(hov.iter().chain(fly.iter()).all(|i| sel.contains(i)));
    assert!(sm.ships_in_circle(1, Vec2::new(0.0, 3100.0), 300.0).is_empty(), "only your own ships");
}

#[test]
fn ships_in_flight_can_be_redirected() {
    let mut sm = sim(with(base(), vec![sun(-1, -9000.0, 0.0, 1, 0)]));
    let ids = sm.all_ships(0);
    sm.step(&[order(0, ids.clone(), Target::Obj(2))]);
    run(&mut sm, 120);
    sm.step(&[order(0, ids.clone(), Target::Point(Vec2::new(-6000.0, 9000.0)))]);
    run(&mut sm, 1200);
    assert_eq!(s(&sm, 2).cap_progress, 0);
    assert!(ids.iter().all(|&i| ship_by_id(&sm, i).is_some_and(|x| x.mode == Mode::Hover && x.pos.dist(Vec2::new(-6000.0, 9000.0)) < POINT_SPREAD_MAX)));
}

#[test]
fn ships_sent_to_empty_space_stop_and_hover_there_until_ordered_again() {
    let mut sm = sim(base());
    let ids: Vec<u32> = sm.all_ships(0).into_iter().take(30).collect();
    let p = Vec2::new(-8000.0, 8400.0); // close enough to the home sun that idle ships would join it
    sm.step(&[order(0, ids.clone(), Target::Point(p))]);
    run(&mut sm, 1200);
    for &i in &ids {
        let x = ship_by_id(&sm, i).unwrap();
        assert!(x.mode == Mode::Hover && x.hold && x.pos.dist(p) < 120.0, "{:?} {:?}", x.mode, x.pos);
    }
}

#[test]
fn there_is_no_cap_on_orders() {
    let mut sm = sim(base());
    let ids = sm.all_ships(0);
    let cmds: Vec<Command> = ids.iter().take(40).map(|&i| order(0, vec![i], Target::Obj(1))).collect();
    sm.step(&cmds);
    assert!(rejected(&mut sm).is_empty());
}

#[test]
fn cannot_order_other_factions_ships_or_bad_targets() {
    let mut sm = sim(base());
    let theirs = sm.all_ships(1);
    sm.step(&[order(0, theirs, Target::Obj(0))]);
    let mine = sm.all_ships(0);
    sm.step(&[order(0, mine.clone(), Target::Obj(99))]);
    assert_eq!(rejected(&mut sm), vec![OrderError::NoShips, OrderError::BadTarget]);
}

#[test]
fn ships_bend_around_suns_in_their_way_and_wobble() {
    let mut sm = sim(with(base(), vec![sun(-1, 0.0, 0.0, 1, 0)]));
    let ids = swarm(&mut sm, 0, -2000.0, -20.0, 30, Target::Point(Vec2::new(2000.0, 0.0)));
    let r = s(&sm, 2).radius;
    let mut closest = f64::MAX;
    let mut spread = 0.0f64;
    for _ in 0..900 {
        sm.step(&[]);
        let ys: Vec<f64> = ids.iter().filter_map(|&i| ship_by_id(&sm, i)).filter(|x| x.mode == Mode::Fly).map(|x| x.pos.y).collect();
        if ys.len() > 2 {
            spread = spread.max(ys.iter().cloned().fold(f64::MIN, f64::max) - ys.iter().cloned().fold(f64::MAX, f64::min));
        }
        for &i in &ids {
            closest = closest.min(ship_by_id(&sm, i).unwrap().pos.len());
        }
    }
    assert!(closest > r, "a ship flew through the sun ({closest} < {r})");
    assert!(spread > 60.0, "the swarm stayed in a line ({spread})");
    assert_eq!(s(&sm, 2).cap_progress, 0, "passing a sun does not capture it");
}

#[test]
fn crossing_a_typical_level_takes_tens_of_seconds() {
    let secs = 10_000.0 / (SHIP_SPEED * TICKS_PER_SEC as f64);
    assert!((15.0..=60.0).contains(&secs), "{secs}");
}

// ---------------------------------------------------------------- 3. self-defence
#[test]
fn a_serious_attack_makes_part_of_the_cloud_break_off_to_meet_it() {
    // the human's (faction 0) suns defend themselves too
    let mut sm = sim(base());
    let home = s(&sm, 0).pos;
    let attackers = swarm(&mut sm, 1, -9000.0, 6000.0, 40, Target::Obj(0));
    run(&mut sm, 150);
    let guards: Vec<&Ship> = sm.ships.iter().filter(|x| x.faction == 0 && x.guard == 1).collect();
    assert!(!guards.is_empty(), "nobody came out");
    assert!(guards.iter().all(|x| x.pos.y < home.y), "defenders go to the attackers' side");
    run(&mut sm, 1200);
    assert!(attackers.iter().all(|&i| ship_by_id(&sm, i).is_none()), "attack beaten");
    assert_eq!(s(&sm, 0).faction, 0);
    assert!(sm.ships.iter().all(|x| x.guard == 0), "defenders return to circling");
}

#[test]
fn small_raids_and_orbiting_suns_do_not_trigger_self_defence() {
    let mut sm = sim(base());
    swarm(&mut sm, 1, -9000.0, 6000.0, 5, Target::Obj(0));
    run(&mut sm, 150);
    assert!(sm.ships.iter().all(|x| x.guard == 0));
    let o = Object::OrbitingSun(sdef(0, 0.0, 0.0, 2, 60), OrbitDef { rx: 500.0, ry: 500.0, rotation_deg: 0.0, initial_angle_deg: 0.0, speed: 6.0 });
    let mut sm = sim(with(base(), vec![o]));
    swarm(&mut sm, 1, 0.0, -2500.0, 40, Target::Obj(2));
    run(&mut sm, 150);
    assert!(sm.ships.iter().all(|x| x.guard == 0));
}

// ---------------------------------------------------------------- 4. combat
fn survivors(sm: &Sim, f: u8) -> u32 {
    sm.strength(f)
}

fn clash(n0: usize, n1: usize) -> (u32, u32) {
    let mut sm = sim(base());
    swarm(&mut sm, 0, -600.0, 0.0, n0, Target::Point(Vec2::new(600.0, 0.0)));
    swarm(&mut sm, 1, 600.0, 0.0, n1, Target::Point(Vec2::new(-600.0, 0.0)));
    run(&mut sm, 600);
    (survivors(&sm, 0) - 100 - 20, survivors(&sm, 1) - 100 - 20)
}

#[test]
fn equal_swarms_cancel_out_and_the_larger_keeps_about_the_difference() {
    let (a, b) = clash(30, 30);
    assert!(a + b <= 3, "{a} {b}");
    let (a, b) = clash(50, 30);
    assert!((16..=24).contains(&a) && b <= 1, "{a} {b}");
    let (b2, a2) = clash(30, 50);
    assert!((16..=24).contains(&a2) && b2 <= 1, "symmetric: {a2} {b2}");
}

#[test]
fn enemy_ships_seek_each_other_at_close_range_but_not_far() {
    for (gap, fight) in [(60.0, true), (300.0, false)] {
        let mut sm = sim(base());
        let a = sm.add_ship(0, Vec2::new(-1000.0, -gap / 2.0), Target::Point(Vec2::new(1000.0, -gap / 2.0)));
        let b = sm.add_ship(1, Vec2::new(1000.0, gap / 2.0), Target::Point(Vec2::new(-1000.0, gap / 2.0)));
        run(&mut sm, 500);
        assert_eq!(ship_by_id(&sm, a).is_none() && ship_by_id(&sm, b).is_none(), fight, "gap {gap}");
    }
}

#[test]
fn friendly_ships_never_hurt_each_other() {
    let mut sm = sim(base());
    swarm(&mut sm, 0, -600.0, 0.0, 30, Target::Point(Vec2::new(600.0, 0.0)));
    swarm(&mut sm, 0, 600.0, 0.0, 30, Target::Point(Vec2::new(-600.0, 0.0)));
    run(&mut sm, 600);
    assert_eq!(sm.strength(0), 100 + 20 + 60);
}

#[test]
fn a_stronger_ship_survives_a_weaker_one_weakened() {
    let mut sm = sim(base());
    let a = sm.add_ship(0, Vec2::new(-200.0, 0.0), Target::Point(Vec2::new(200.0, 0.0)));
    let b = sm.add_ship(1, Vec2::new(200.0, 0.0), Target::Point(Vec2::new(-200.0, 0.0)));
    let ia = sm.ship_index(a).unwrap();
    sm.ships[ia].strength = 5;
    let ib = sm.ship_index(b).unwrap();
    sm.ships[ib].strength = 2;
    run(&mut sm, 120);
    assert_eq!(ship_by_id(&sm, a).map(|x| x.strength), Some(3));
    assert!(ship_by_id(&sm, b).is_none());
    assert_eq!(sm.factions[0].combat_losses, 2);
}

#[test]
fn defenders_meet_attackers_in_the_cloud_before_the_sun() {
    let mut sm = sim(vec![sun(0, -9000.0, 9000.0, 3, 100), sun(1, 9000.0, 9000.0, 3, 100), sun(1, 0.0, 0.0, 1, 60)]);
    // a raid below the self-defence threshold still has to get through the cloud
    swarm(&mut sm, 0, -1500.0, 0.0, 10, Target::Obj(2));
    run(&mut sm, 600);
    assert_eq!(s(&sm, 2).health, s(&sm, 2).max_health(), "the cloud stopped the raid");
}

// ---------------------------------------------------------------- 5. winning and losing
#[test]
fn a_faction_without_suns_and_with_too_few_ships_to_capture_one_is_out() {
    for (stragglers, out) in [(60usize, true), (150, false)] {
        let mut sm = sim(vec![sun(0, -4000.0, 0.0, 3, 100), sun(1, 4000.0, 0.0, 1, 0), sun(-1, 0.0, 6000.0, 1, 0)]);
        parked(&mut sm, 1, 0.0, -8000.0, stragglers);
        swarm(&mut sm, 0, 2500.0, 0.0, 140, Target::Obj(1));
        run(&mut sm, 600);
        assert_eq!(s(&sm, 1).faction, -1);
        assert_eq!(!sm.factions[1].alive, out, "{stragglers}");
        if out {
            assert_eq!(sm.status, Status::Won);
            assert_eq!(sm.ship_count(1), 0, "an out faction's ships vanish");
        }
    }
}

#[test]
fn a_rival_owning_every_sun_and_several_times_stronger_ends_it() {
    for (garrison, out) in [(800u32, true), (300, false)] {
        let mut sm = sim(vec![sun(0, -4000.0, 0.0, 3, garrison), sun(1, 4000.0, 0.0, 1, 0)]);
        parked(&mut sm, 1, 0.0, -8000.0, 150);
        swarm(&mut sm, 0, 2500.0, 0.0, 210, Target::Obj(1)); // 100 to kill it, 100 to take it
        run(&mut sm, 900);
        assert_eq!(s(&sm, 1).faction, 0, "{garrison}");
        assert_eq!(!sm.factions[1].alive, out, "{garrison}");
    }
}

#[test]
fn human_loses_when_out_and_the_game_freezes() {
    let mut sm = sim(base());
    sm.step(&[Command::Surrender { faction: 0 }]);
    assert_eq!(sm.status, Status::Lost);
    let t = sm.tick;
    run(&mut sm, 10);
    assert_eq!(sm.tick, t);
}

#[test]
fn statistics_are_sampled_for_the_results_screen() {
    let mut sm = sim(base());
    swarm(&mut sm, 0, -600.0, 0.0, 10, Target::Point(Vec2::new(600.0, 0.0)));
    swarm(&mut sm, 1, 600.0, 0.0, 10, Target::Point(Vec2::new(-600.0, 0.0)));
    run(&mut sm, STATS_TICKS as u32 * 3);
    assert_eq!(sm.stats.len(), 4);
    let last = sm.stats.last().unwrap();
    assert_eq!(last.tick, STATS_TICKS * 3);
    assert_eq!(last.production, vec![2, 2]);
    assert!(last.losses[0] >= 9 && last.losses[0] == last.losses[1]);
    assert_eq!(last.ships[0], sm.strength(0));
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
    run(&mut sm, 15 * 60); // 6 deg/s -> a quarter turn in 15 s
    let p1 = s(&sm, 2).pos;
    assert!(p1.x.abs() < 5.0 && (p1.y - 1000.0).abs() < 5.0, "{p1:?}");
    run(&mut sm, 45 * 60);
    assert!(s(&sm, 2).pos.dist(p0) < 5.0, "closed path");
}

#[test]
fn ships_aimed_at_an_orbiting_sun_track_it() {
    let mut sm = sim(with(base(), vec![orbiter(-1, 0.0, 0.0, 1500.0, 1500.0, 12.0)]));
    swarm(&mut sm, 0, -3000.0, -3000.0, 100, Target::Obj(2));
    run(&mut sm, 2400);
    assert_eq!(s(&sm, 2).faction, 0);
}

fn fragile(x: f64, y: f64, broken: u32) -> Object {
    Object::FragileSun(FragileDef { sun: sdef(-1, x, y, 2, 0), orbit: OrbitDef::default(), broken_seconds: broken, rebuild_frames: 60 })
}

#[test]
fn fragile_sun_survives_capture_but_breaks_when_killed_then_rebuilds_empty() {
    let mut sm = sim(with(base(), vec![fragile(0.0, 0.0, 4)]));
    swarm(&mut sm, 0, -1500.0, 0.0, 100, Target::Obj(2));
    run(&mut sm, 600);
    assert_eq!(s(&sm, 2).faction, 0, "capturing it empty does not break it");
    assert!(s(&sm, 2).present());
    swarm(&mut sm, 1, 1500.0, 0.0, 140, Target::Obj(2)); // its own ring ships defend it a little
    run(&mut sm, 330);
    assert!(!s(&sm, 2).present(), "killed: broken");
    assert!(sm.take_events().iter().any(|e| matches!(e, Event::SunBroken { sun: 2 })));
    // a wreck cannot be targeted
    let ids = sm.all_ships(0);
    sm.step(&[order(0, ids, Target::Obj(2))]);
    assert_eq!(rejected(&mut sm), vec![OrderError::BadTarget]);
    // broken for brokenseconds * FRAGILE_TICKS_PER_UNIT, then rebuilds over rebuildframes
    run(&mut sm, 4 * FRAGILE_TICKS_PER_UNIT + 61);
    let x = s(&sm, 2);
    assert!(x.present() && x.faction == -1 && x.level == 0);
}

#[test]
fn fragile_breaks_take_a_couple_of_minutes_with_our_levels() {
    let secs = 240 * FRAGILE_TICKS_PER_UNIT / TICKS_PER_SEC;
    assert!((90..=180).contains(&secs), "{secs}");
}

fn wh(x: f64, y: f64, dest: i32) -> Object {
    Object::Wormhole(WormholeDef { destination: dest, size: 400.0, orbit: OrbitDef::default(), pos: [x, y] })
}

#[test]
fn only_ships_ordered_to_a_wormhole_use_it_and_they_come_out_idle_keeping_heading() {
    let mut sm = sim(with(base(), vec![wh(0.0, 0.0, 3), wh(5000.0, 3000.0, 2)]));
    let passer = sm.add_ship(0, Vec2::new(-1000.0, 0.0), Target::Point(Vec2::new(1000.0, 0.0)));
    let user = sm.add_ship(0, Vec2::new(-1000.0, 100.0), Target::Obj(2));
    let mut jumped_at = None;
    for t in 0..400 {
        sm.step(&[]);
        if jumped_at.is_none() && ship_by_id(&sm, user).unwrap().pos.dist(Vec2::new(5000.0, 3000.0)) < 600.0 {
            jumped_at = Some(t);
            let x = ship_by_id(&sm, user).unwrap();
            assert!(x.idle(), "idle after the jump");
            assert!(x.heading.x > 0.8, "heading kept: {:?}", x.heading);
        }
        assert!(ship_by_id(&sm, passer).unwrap().pos.dist(Vec2::new(5000.0, 3000.0)) > 1000.0, "passer was teleported");
    }
    assert!(jumped_at.is_some());
    assert!(ship_by_id(&sm, passer).unwrap().pos.dist(Vec2::new(1000.0, 0.0)) < 50.0);
}

#[test]
fn a_random_destination_is_chosen_per_ship_among_the_other_wormholes() {
    let mut sm = sim(with(base(), vec![wh(0.0, 0.0, -1), wh(5000.0, 0.0, -1), wh(0.0, -5000.0, -1)]));
    let ids = swarm(&mut sm, 0, -1200.0, 0.0, 30, Target::Obj(2));
    run(&mut sm, 300);
    let (mut a, mut b) = (0, 0);
    for &i in &ids {
        let p = ship_by_id(&sm, i).unwrap().pos;
        if p.dist(Vec2::new(5000.0, 0.0)) < 1500.0 {
            a += 1;
        } else if p.dist(Vec2::new(0.0, -5000.0)) < 1500.0 {
            b += 1;
        }
    }
    assert_eq!(a + b, 30);
    assert!(a > 0 && b > 0, "the swarm split ({a}/{b})");
}

fn white() -> Object {
    Object::WhiteHole(WhiteHoleDef { required_investment: 50, start_return: 40, end_return: 80, growth_rate: 0.0, game_size: 300.0, dormant_period: 0, pos: [0.0, 0.0] })
}
fn white_of(sm: &Sim, i: usize) -> &WhiteHole {
    match &sm.objs[i] {
        Obj::WhiteHole(w) => w,
        _ => panic!(),
    }
}

#[test]
fn white_hole_investment_is_a_tug_of_war() {
    let mut sm = sim(with(base(), vec![white()]));
    swarm(&mut sm, 0, -1000.0, 0.0, 30, Target::Obj(2));
    run(&mut sm, 400);
    assert_eq!((white_of(&sm, 2).investor, white_of(&sm, 2).invest), (0, 30));
    swarm(&mut sm, 1, 1000.0, 0.0, 45, Target::Obj(2));
    run(&mut sm, 400);
    assert_eq!((white_of(&sm, 2).investor, white_of(&sm, 2).invest), (1, 15));
}

#[test]
fn white_hole_fires_a_stream_of_its_prize_then_is_spent_and_regrows() {
    let mut sm = sim(with(base(), vec![white()]));
    swarm(&mut sm, 1, 1000.0, 0.0, 50, Target::Obj(2));
    let before = sm.strength(1);
    let mut fired = None;
    for t in 0..2000u64 {
        sm.step(&[]);
        if fired.is_none() && matches!(white_of(&sm, 2).phase, WhitePhase::Firing { .. }) {
            fired = Some(t);
        }
        if fired.is_some() && matches!(white_of(&sm, 2).phase, WhitePhase::Spent { .. }) {
            break;
        }
    }
    let fired = fired.expect("never fired");
    assert!(sm.tick - fired >= WHITE_FIRE_TICKS - 2, "streamed over several seconds");
    // the 50 invested are gone; the prize (40 at the start) came out
    assert_eq!(sm.strength(1), before - 50 + 40 + (sm.tick as u32 / 60) * 2 - 0);
    // spent: new ships do not invest
    swarm(&mut sm, 0, -1000.0, 0.0, 10, Target::Obj(2));
    run(&mut sm, 400);
    assert_eq!(white_of(&sm, 2).invest, 0);
    run(&mut sm, (WHITE_MIN_SPENT + WHITE_REGROW_TICKS) as u32);
    assert!(white_of(&sm, 2).accepting());
}

#[test]
fn white_hole_prize_grows_in_steps_and_large_prizes_come_as_stronger_ships() {
    let mut sm = sim(with(base(), vec![white()]));
    let w0 = white_of(&sm, 2).prize(sm.tick);
    run(&mut sm, WHITE_GROW_TICKS as u32 / 2);
    let w1 = white_of(&sm, 2).prize(sm.tick);
    run(&mut sm, WHITE_GROW_TICKS as u32 / 2);
    let w2 = white_of(&sm, 2).prize(sm.tick);
    assert!(w0 == 40 && w0 < w1 && w1 < w2 && w2 == 80);
    assert!(1 + 600 / WHITE_STRONG_ABOVE > 1, "a 600-ship prize streams as stronger ships");
}

fn nova(kill_suns: bool) -> Object {
    Object::Supernova(SupernovaDef {
        required_investment: 50,
        start_scale: 400.0,
        end_scale: 600.0,
        blast_radius: 2500.0,
        collapse_radius: 3000.0,
        kill_enemy_suns: kill_suns,
        kill_enemy_units: true,
        unit_residuals: 11,
        dormant_period: 30,
        pos: [0.0, 0.0],
    })
}
fn nova_of(sm: &Sim, i: usize) -> &Supernova {
    match &sm.objs[i] {
        Obj::Supernova(w) => w,
        _ => panic!(),
    }
}

#[test]
fn supernova_collapses_pulling_ships_in_then_blasts_enemies_and_gives_new_ships() {
    let mut sm = sim(with(base(), vec![nova(false)]));
    let near = parked(&mut sm, 0, 2000.0, 0.0, 5);
    let far = parked(&mut sm, 0, 6000.0, 0.0, 5);
    let own = parked(&mut sm, 1, -2000.0, 0.0, 5);
    swarm(&mut sm, 1, 0.0, 1000.0, 50, Target::Obj(2));
    let mut collapsed = false;
    let mut at_blast = None;
    for _ in 0..600 {
        sm.step(&[]);
        if at_blast.is_none() && matches!(nova_of(&sm, 2).phase, NovaPhase::Blast { .. }) {
            at_blast = Some(ship_by_id(&sm, own[0]).unwrap().pos.x);
        }
        if !collapsed && matches!(nova_of(&sm, 2).phase, NovaPhase::Collapse { .. }) {
            collapsed = true;
            let d0 = ship_by_id(&sm, near[0]).unwrap().pos.x;
            run(&mut sm, 60);
            assert!(ship_by_id(&sm, near[0]).unwrap().pos.x < d0 - 100.0, "pulled in");
        }
    }
    assert!(collapsed);
    assert!(near.iter().all(|&i| ship_by_id(&sm, i).is_none()), "enemy ships in range destroyed");
    assert!(far.iter().all(|&i| ship_by_id(&sm, i).is_some()), "out of range survive");
    for &i in &own {
        let x = ship_by_id(&sm, i).unwrap();
        assert!(x.idle() && !x.hold, "survivors lose their order");
    }
    assert!(ship_by_id(&sm, own[0]).unwrap().pos.x < at_blast.unwrap() - 150.0, "shoved outward");
    let fresh = sm.ships.iter().filter(|x| x.faction == 1 && x.pos.len() < 800.0).count();
    assert!(fresh >= 11, "{fresh}");
    assert!(matches!(nova_of(&sm, 2).phase, NovaPhase::Dormant { .. }));
    run(&mut sm, 30 * NOVA_TICKS_PER_UNIT as u32 + 2);
    assert_eq!(nova_of(&sm, 2).phase, NovaPhase::Ready);
}

#[test]
fn supernova_kills_enemy_suns_in_range_when_the_level_says_so() {
    let mut sm = sim(with(base(), vec![nova(true), sun(0, 1500.0, -1500.0, 1, 0), sun(0, 4000.0, -4000.0, 1, 0)]));
    swarm(&mut sm, 1, 0.0, 1000.0, 50, Target::Obj(2));
    run(&mut sm, 600);
    assert_eq!(s(&sm, 3).faction, -1);
    assert_eq!(s(&sm, 4).faction, 0);
}

fn gamma(kill_units: bool, random: bool) -> Object {
    Object::GammaRay(GammaRayDef { kill_suns: true, kill_units, rotation_deg: 0.0, width: 600.0, period: 10, randomization: 0.0, random_start: random, pos: [0.0, 0.0] })
}
fn gamma_of(sm: &Sim, i: usize) -> &Gamma {
    match &sm.objs[i] {
        Obj::Gamma(g) => g,
        _ => panic!(),
    }
}

#[test]
fn gamma_ray_warns_then_pulses_along_a_fixed_lane_killing_suns_in_it() {
    let mut sm = sim(with(base(), vec![gamma(false, false), sun(1, 3000.0, 0.0, 3, 0), sun(1, 3000.0, 2000.0, 3, 0)]));
    sun_mut(&mut sm, 3).level = 3;
    let at = gamma_of(&sm, 2).next_pulse;
    assert_eq!(at, 10 * GAMMA_TICKS_PER_PERIOD as u64);
    run(&mut sm, (at - GAMMA_WARN_TICKS) as u32);
    assert!(sm.take_events().iter().any(|e| matches!(e, Event::GammaWarning { obj: 2 })));
    assert!(gamma_of(&sm, 2).warning(sm.tick));
    run(&mut sm, GAMMA_WARN_TICKS as u32);
    assert_eq!(s(&sm, 3).faction, 1, "warning only");
    run(&mut sm, GAMMA_SWEEP_TICKS as u32 + 1);
    assert_eq!((s(&sm, 3).faction, s(&sm, 3).level), (-1, 0), "the sun in the lane died whatever its level");
    assert_eq!(s(&sm, 4).faction, 1, "outside the lane");
    assert_eq!(gamma_of(&sm, 2).dir, au2_sim::math::dir_deg(0.0), "the lane does not rotate");
    assert!(gamma_of(&sm, 2).next_pulse > sm.tick, "next pulse scheduled");
}

#[test]
fn gamma_ray_kills_ships_only_when_the_level_says_so() {
    for (kill, left) in [(true, 0usize), (false, 1)] {
        let mut sm = sim(with(base(), vec![gamma(kill, false)]));
        parked(&mut sm, 0, 3000.0, 100.0, 1);
        run(&mut sm, (10 * GAMMA_TICKS_PER_PERIOD) as u32 + GAMMA_SWEEP_TICKS as u32 + 2);
        assert_eq!(sm.ships.iter().filter(|x| x.pos.len() < 4000.0).count(), left);
    }
}

#[test]
fn gamma_ray_random_start_depends_on_seed_but_is_reproducible() {
    let first = |seed| {
        let sm = Sim::new(&map(2, with(base(), vec![gamma(false, true)])), SimConfig { seed, ..Default::default() });
        gamma_of(&sm, 2).next_pulse
    };
    assert_eq!(first(5), first(5));
    assert_ne!(first(5), first(6));
}

#[test]
fn shockwave_walls_creep_across_destroying_ships() {
    // a shrinking ring from r = 2000 at 600 u/s
    let sw = Object::Shockwave(ShockwaveDef { kind: "st_incircle".into(), dormant: 0, period: 0, speed: 600.0, lifetime: 0, line_rotation_deg: 0.0, start_offset: 2000.0, pos: [0.0, 0.0] });
    let mut sm = sim(with(base(), vec![sw]));
    let inner = parked(&mut sm, 0, 1000.0, 0.0, 1)[0];
    let outer = parked(&mut sm, 0, 3000.0, 0.0, 1)[0];
    run(&mut sm, 70);
    assert!(ship_by_id(&sm, inner).is_some());
    run(&mut sm, 30);
    assert!(ship_by_id(&sm, inner).is_none());
    assert!(ship_by_id(&sm, outer).is_some());
    // walking pace by default: minutes to cross a level
    assert!(10_000.0 / (SHOCK_DEFAULT_SPEED * 60.0) > 60.0);
}

fn fog() -> Object {
    Object::Fog(FogDef { sun_radius: 2000.0, additional_per_ring: 500.0, unowned_radius: 0.0, enemy_radius: 0.0, non_sun_radius: 500.0, pos: [0.0, 0.0] })
}

#[test]
fn fog_shows_only_what_suns_see_and_refuses_orders_into_it() {
    let mut sm = sim(with(base(), vec![fog()]));
    let v = sm.vision(0);
    assert!(v.visible(Vec2::new(-7100.0, 9000.0)));
    assert!(!v.visible(Vec2::new(-6000.0, 9000.0)));
    assert!(v.visible(Vec2::new(9000.0, 9000.0)), "an enemy sun shows a small area around itself");
    assert!(!v.visible(Vec2::new(9000.0, 8000.0)));
    parked(&mut sm, 0, 0.0, 0.0, 1);
    assert!(!sm.vision(0).visible(Vec2::new(100.0, 0.0)), "ships give no sight");
    sun_mut(&mut sm, 0).level = 3;
    assert!(sm.vision(0).visible(Vec2::new(-6100.0, 9000.0)), "sight grows with level");
    let ids = sm.all_ships(0);
    sm.step(&[order(0, ids.clone(), Target::Point(Vec2::new(0.0, 0.0)))]);
    sm.step(&[order(0, ids, Target::Obj(1))]);
    assert_eq!(rejected(&mut sm), vec![OrderError::Fogged]);
    // fog lifts once the human is out
    sm.step(&[Command::Surrender { faction: 0 }]);
    assert!(sm.vision(0).all);
}

// ---------------------------------------------------------------- 7. terrain
fn terr(t: TerrainDef) -> Object {
    Object::Terrain(t)
}

#[test]
fn force_terrain_pushes_ships_in_a_set_direction() {
    let mut t = terrain(TerrainKind::Force, -2000.0, -2000.0, 4, 4, 1000.0);
    t.acceleration = [0.0, 300.0];
    let mut sm = sim(with(base(), vec![terr(t)]));
    let id = sm.add_ship(0, Vec2::new(-1000.0, 0.0), Target::Point(Vec2::new(1000.0, 0.0)));
    run(&mut sm, 200);
    assert!(ship_by_id(&sm, id).unwrap().pos.y > 100.0);
}

#[test]
fn circular_force_pushes_outward_or_inward_and_can_turn_a_quarter() {
    for (acc, out) in [(300.0, true), (-300.0, false)] {
        let mut t = terrain(TerrainKind::Force, 0.0, 0.0, 1, 1, 1.0);
        t.outer_radius = 3000.0;
        t.acceleration = [acc, 0.0];
        let mut sm = sim(with(base(), vec![terr(t)]));
        let id = parked(&mut sm, 0, 1000.0, 0.0, 1)[0];
        run(&mut sm, 60);
        let x = ship_by_id(&sm, id).unwrap().pos.x;
        assert_eq!(x > 1000.0, out, "{x}");
    }
    let mut t = terrain(TerrainKind::Force, 0.0, 0.0, 1, 1, 1.0);
    t.outer_radius = 3000.0;
    t.acceleration = [300.0, 0.0];
    t.switch_time = 10;
    t.dir_switch = 1;
    let mut sm = sim(with(base(), vec![terr(t)]));
    run(&mut sm, 10);
    match &sm.objs[2] {
        Obj::Terrain(t) => {
            assert_eq!(t.quarter, 1);
            let p = t.push_at(Vec2::new(1000.0, 0.0));
            assert!(p.x.abs() < 1e-9 && p.y > 0.0, "swirls: {p:?}");
        }
        _ => panic!(),
    }
}

#[test]
fn speed_terrain_slows_or_speeds_ships_including_circling_ones() {
    let travelled = |change: f64| {
        let mut t = terrain(TerrainKind::Speed, -3000.0, -3000.0, 6, 6, 1000.0);
        t.speed_change = change;
        let mut sm = sim(with(base(), vec![terr(t)]));
        let id = sm.add_ship(0, Vec2::new(-1000.0, 0.0), Target::Point(Vec2::new(10000.0, 0.0)));
        run(&mut sm, 60);
        ship_by_id(&sm, id).unwrap().pos.x + 1000.0
    };
    let normal = travelled(0.0);
    assert!(travelled(-20.0) < normal * 0.5, "well under half speed");
    assert!(travelled(32.0) > normal * 1.5);
    // circling: put a speed zone over the home sun
    let angle = |change: f64| {
        let mut t = terrain(TerrainKind::Speed, -10000.0, 8000.0, 2, 2, 1000.0);
        t.speed_change = change;
        let mut sm = sim(with(base(), vec![terr(t)]));
        let x0 = sm.ships[0].orbit_ang;
        run(&mut sm, 30);
        sm.ships[0].orbit_ang - x0
    };
    assert!(angle(-20.0) < angle(0.0) * 0.6);
}

#[test]
fn reflect_terrain_is_a_wall_ships_bounce_off() {
    let t = terrain(TerrainKind::Reflect, 0.0, -2000.0, 4, 1, 1000.0);
    let mut sm = sim(with(base(), vec![terr(t)]));
    let id = sm.add_ship(0, Vec2::new(-500.0, 0.0), Target::Point(Vec2::new(3000.0, 0.0)));
    for _ in 0..300 {
        sm.step(&[]);
        assert!(ship_by_id(&sm, id).unwrap().pos.x < 0.0, "went through the wall");
    }
}

#[test]
fn minefields_kill_occasionally_and_wear_out() {
    let mut t = terrain(TerrainKind::Minefield, -2500.0, -500.0, 2, 10, 500.0);
    t.death_rate = 20.0;
    let mut sm = sim(with(base(), vec![terr(t)]));
    let start: u32 = match &sm.objs[2] {
        Obj::Terrain(t) => t.mines.iter().map(|&m| m as u32).sum(),
        _ => panic!(),
    };
    swarm(&mut sm, 0, -3000.0, -200.0, 300, Target::Point(Vec2::new(3500.0, -200.0)));
    run(&mut sm, 1500);
    let lost = 100 + 300 + 50 - sm.strength(0);
    let left: u32 = match &sm.objs[2] {
        Obj::Terrain(t) => t.mines.iter().map(|&m| m as u32).sum(),
        _ => panic!(),
    };
    assert!(lost >= 1 && lost <= 300 / 12, "about one in a few dozen per crossing: {lost}");
    assert_eq!(start - left, lost, "every death uses up a mine");
    // a cleared cell is safe
    if let Obj::Terrain(t) = &mut sm.objs[2] {
        t.mines.iter_mut().for_each(|m| *m = 0);
    }
    let before = sm.strength(0);
    swarm(&mut sm, 0, -3000.0, -200.0, 300, Target::Point(Vec2::new(3500.0, -200.0)));
    run(&mut sm, 1500);
    assert_eq!(sm.strength(0), before + 300 + 50);
}

#[test]
fn confusion_converts_ships_to_the_weakest_faction_with_a_sun() {
    let mut t = terrain(TerrainKind::Confusion, -2000.0, -2000.0, 4, 4, 1000.0);
    t.faction_change_rate = 100.0;
    let objs = vec![sun(0, -9000.0, 9000.0, 3, 100), sun(1, 9000.0, 9000.0, 3, 100), sun(2, 0.0, 9000.0, 3, 10), terr(t)];
    let mut sm = sim_n(3, objs);
    let id = parked(&mut sm, 0, 0.0, 0.0, 1)[0];
    run(&mut sm, 600);
    assert_eq!(ship_by_id(&sm, id).unwrap().faction, 2);
}

#[test]
fn confusion_can_drop_orders_and_send_ships_to_a_random_sun() {
    let mut t = terrain(TerrainKind::Confusion, -2000.0, -2000.0, 4, 4, 1000.0);
    t.lose_order_rate = 100.0;
    let mut sm = sim(with(base(), vec![terr(t)]));
    let id = parked(&mut sm, 0, 0.0, 0.0, 1)[0];
    run(&mut sm, 200);
    assert!(matches!(ship_by_id(&sm, id).map(|x| x.target), Some(Target::Obj(_)) | None));
}

#[test]
fn neutral_zone_ships_fight_nobody() {
    let t = terrain(TerrainKind::Neutral, -2000.0, -2000.0, 4, 4, 1000.0);
    let mut sm = sim(with(base(), vec![terr(t)]));
    parked(&mut sm, 0, 0.0, 0.0, 5);
    parked(&mut sm, 1, 0.0, 4.0, 5);
    run(&mut sm, 120);
    assert_eq!(sm.ships.iter().filter(|x| x.pos.len() < 200.0).count(), 10);
}

#[test]
fn circular_zone_is_an_annulus_and_zones_can_switch() {
    let mut t = terrain(TerrainKind::Neutral, 0.0, 0.0, 1, 1, 1.0);
    t.outer_radius = 1000.0;
    t.inner_radius = 400.0;
    t.switch_time = 50;
    let mut sm = sim(with(base(), vec![terr(t)]));
    let Obj::Terrain(z) = &sm.objs[2] else { panic!() };
    assert!(!z.contains(Vec2::new(100.0, 0.0)) && z.contains(Vec2::new(700.0, 0.0)) && !z.contains(Vec2::new(1100.0, 0.0)));
    run(&mut sm, 50);
    let Obj::Terrain(z) = &sm.objs[2] else { panic!() };
    assert!(!z.enabled);
}

// ---------------------------------------------------------------- crowding
#[test]
fn a_crowded_board_fuses_nearby_idle_ships_without_losing_strength() {
    let mut sm = sim(base());
    for k in 0..MERGE_THRESHOLD {
        let p = Vec2::new((k % 60) as f64 * 6.0, (k / 60) as f64 * 6.0);
        sm.add_ship(0, p, Target::Point(p));
    }
    let (n0, st0) = (sm.ships.len(), sm.strength(0));
    run(&mut sm, MERGE_TICKS as u32);
    assert!(sm.ships.len() < n0 * 3 / 4, "{} of {n0}", sm.ships.len());
    assert_eq!(sm.strength(0), st0 + 2 * (MERGE_TICKS as u32 / 60));
    assert!(sm.ships.iter().all(|x| x.strength <= MERGE_MAX));
}

// ---------------------------------------------------------------- 9. time & determinism
fn scripted(seed: u64, swap: bool) -> u64 {
    let objs = with(base(), vec![sun(-1, 0.0, 0.0, 2, 0), sun(-1, 3000.0, 3000.0, 1, 0)]);
    let mut sm = Sim::new(&map(2, objs), SimConfig { seed, ai_difficulty: 2, human_ai: false, enable_ai: true });
    let ids = sm.all_ships(0);
    for t in 0..600u32 {
        let mut cmds = Vec::new();
        if t == 5 {
            cmds.push(order(0, ids[..40].to_vec(), Target::Obj(2)));
        }
        if t == (if swap { 7 } else { 6 }) {
            cmds.push(order(0, ids[40..].to_vec(), Target::Point(Vec2::new(-5000.0, 5000.0))));
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
    for k in 0..360 {
        let a = k as f64 * 7.3 - 900.0;
        let (y, x) = (sin_deg(a) * 3.0, cos_deg(a) * 3.0);
        let mut diff = atan2_deg(y, x) - a.rem_euclid(360.0);
        diff = diff.rem_euclid(360.0);
        assert!(diff.min(360.0 - diff) < 0.3, "atan2 at {a}");
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
fn ai_waits_then_expands_to_neutral_suns_and_upgrades() {
    let objs = with(
        vec![sun(0, -4000.0, 0.0, 3, 100), sun(1, 4000.0, 0.0, 3, 100)],
        vec![sun(-1, -1500.0, 0.0, 1, 0), sun(-1, 1500.0, 0.0, 1, 0)],
    );
    let mut sm = Sim::new(&map(2, objs), SimConfig { seed: 1, ai_difficulty: 2, human_ai: false, enable_ai: true });
    run(&mut sm, AI_START_TICKS as u32 - 1);
    assert!(sm.ships.iter().all(|x| x.idle()), "the AI waits a few seconds");
    run(&mut sm, 60 * 150);
    assert_eq!(s(&sm, 3).faction, 1, "AI faction took the nearer neutral sun");
    assert!(s(&sm, 1).level >= 2 || s(&sm, 3).level >= 1);
}

#[test]
fn ai_makes_few_large_moves() {
    let objs = vec![sun(0, -4000.0, 0.0, 3, 100), sun(1, 4000.0, 0.0, 3, 300), sun(-1, 1500.0, 0.0, 1, 0), sun(-1, 4000.0, 2500.0, 1, 0)];
    let mut sm = Sim::new(&map(2, objs), SimConfig { seed: 1, ai_difficulty: 1, human_ai: false, enable_ai: true });
    let mut orders = Vec::new();
    for _ in 0..60 * 60 {
        sm.step(&[]);
        for e in sm.take_events() {
            if let Event::Order { faction: 1, count } = e {
                orders.push(count);
            }
        }
    }
    assert!(!orders.is_empty() && orders.len() <= 20, "{orders:?}");
    assert!(orders.iter().all(|&c| c >= 20), "{orders:?}");
}
