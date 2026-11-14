use au2_map::*;
use std::path::PathBuf;

fn levels_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels")
}

#[test]
fn loads_every_level_in_the_index() {
    let dir = levels_dir();
    let index = load_index(&dir).expect("levels.txt");
    assert!(index.len() >= 30, "index unexpectedly small: {}", index.len());
    let mut failures = Vec::new();
    for e in &index {
        if let Err(err) = load_level(&dir, e) {
            failures.push(err.to_string());
        }
    }
    assert!(failures.is_empty(), "level load failures:\n{}", failures.join("\n"));
}

#[test]
fn loads_every_au2_file_on_disk() {
    let dir = levels_dir();
    let index = load_index(&dir).unwrap();
    let mut n = 0;
    for ent in std::fs::read_dir(&dir).unwrap() {
        let p = ent.unwrap().path();
        if p.extension().map(|e| e == "au2").unwrap_or(false) {
            load_map(&p).unwrap_or_else(|e| panic!("{e}"));
            let stem = p.file_stem().unwrap().to_string_lossy().to_string();
            assert!(index.iter().any(|e| e.stem == stem), "{stem} not listed in levels.txt");
            n += 1;
        }
    }
    assert_eq!(n, index.len(), "index entries and .au2 files differ");
}

#[test]
fn dawn_twin_contents() {
    let m = load_level(&levels_dir(), &load_index(&levels_dir()).unwrap().iter().find(|e| e.stem == "dawn_twin").unwrap().clone()).unwrap();
    assert_eq!(m.factions.len(), 2);
    assert_eq!(m.objects.len(), 7);
    match &m.objects[0] {
        Object::Sun(s) => {
            assert_eq!(s.faction, 0);
            assert_eq!(s.max_power, 2);
            assert_eq!(s.starting_units, 100);
            assert_eq!(s.pos, [-3500.0, 0.0]);
        }
        o => panic!("{o:?}"),
    }
}

#[test]
fn special_objects_are_parsed() {
    let d = levels_dir();
    let idx = load_index(&d).unwrap();
    let load = |stem: &str| load_level(&d, idx.iter().find(|e| e.stem == stem).unwrap()).unwrap();
    let count = |m: &Map, f: fn(&Object) -> bool| m.objects.iter().filter(|o| f(o)).count();
    assert!(count(&load("tidework_carousel"), |o| matches!(o, Object::OrbitingSun(..))) == 4);
    assert!(count(&load("glasshouse_brittle"), |o| matches!(o, Object::FragileSun(_))) == 6);
    assert!(count(&load("gateways_doorway"), |o| matches!(o, Object::Wormhole(_))) == 2);
    assert!(count(&load("blastworks_fuse"), |o| matches!(o, Object::Supernova(_))) == 1);
    assert!(count(&load("blastworks_scythe"), |o| matches!(o, Object::GammaRay(_))) == 1);
    assert!(count(&load("deep_murk"), |o| matches!(o, Object::Fog(_))) == 1);
    let t = load("currents_rapids");
    let force = t.objects.iter().find_map(|o| if let Object::Terrain(t) = o { Some(t) } else { None }).unwrap();
    assert_eq!(force.kind, TerrainKind::Force);
    assert_eq!(force.data.len(), (force.rows * force.cols) as usize);
    assert_eq!(force.acceleration, [-30.0, 0.0]);
    assert!(count(&load("dawn_quartet"), |o| matches!(o, Object::Unknown(_))) == 0);
}

fn err_of(text: &str) -> MapError {
    parse_map(text).expect_err("should fail")
}

const GOOD: &str = "{\n factions(orderedarray<aumapfaction>)=[2,{\n factionobject(faction)=faction={id(factionid)=fi_player}\n},{\n factionobject(faction)=faction={id(factionid)=fi_player2}\n}],\n objects(orderedarray<aumapobject>)=[2,sun={\n faction(faction)=0,\n power(s4)=1,\n maxpower(s4)=2,\n mapposition(vector2)={x(float)=0,y(float)=0}\n},sun={\n faction(faction)=1,\n power(s4)=1,\n maxpower(s4)=2,\n mapposition(vector2)={x(float)=9,y(float)=0}\n}]\n}";

#[test]
fn minimal_level_parses_and_unknown_fields_are_ignored() {
    assert!(parse_map(GOOD).is_ok());
    let extra = GOOD.replace("power(s4)=1,", "power(s4)=1,\n futurefield(s4)=7,\n");
    assert!(parse_map(&extra).is_ok());
    let unk = GOOD.replace("objects(orderedarray<aumapobject>)=[2,", "objects(orderedarray<aumapobject>)=[3,newthing={a(s4)=1},");
    let m = parse_map(&unk).unwrap();
    assert!(matches!(m.objects[0], Object::Unknown(_)));
}

#[test]
fn errors_have_line_numbers() {
    // missing closing brace
    let e = err_of(&GOOD[..GOOD.len() - 1]);
    assert!(e.line >= 8, "{e}");
    assert!(e.message.contains("end of file") || e.message.contains("missing"), "{e}");

    // malformed number on line 10
    let e = err_of(&GOOD.replace("maxpower(s4)=2,\n mapposition(vector2)={x(float)=0", "maxpower(s4)=2x2,\n mapposition(vector2)={x(float)=0"));
    assert_eq!(e.line, 10, "{e}");

    // wrong array count
    let e = err_of(&GOOD.replace("[2,sun=", "[5,sun="));
    assert!(e.message.contains("declares 5"), "{e}");
    assert_eq!(e.line, 7, "{e}");

    // faction out of range
    let e = err_of(&GOOD.replacen("faction(faction)=1", "faction(faction)=3", 1));
    assert!(e.message.contains("out of range"), "{e}");
    assert_eq!(e.line, 12, "{e}");

    // missing position
    let e = err_of(&GOOD.replacen("mapposition(vector2)={x(float)=0,y(float)=0}", "", 1));
    assert!(e.message.contains("mapposition"), "{e}");

    // trailing junk
    let e = err_of(&format!("{GOOD}\n}}"));
    assert_eq!(e.line, 19, "{e}");

    // display includes line
    assert!(e.to_string().contains("line 19"));
}

#[test]
fn wormhole_validation() {
    let wh = |dest: i32| {
        GOOD.replace(
            "objects(orderedarray<aumapobject>)=[2,",
            &format!("objects(orderedarray<aumapobject>)=[4,wormhole={{\ndestinationwormhole(s4)={dest},\nsize(s4)=300,\nmapposition(vector2)={{x(float)=1,y(float)=1}}\n}},wormhole={{\ndestinationwormhole(s4)=0,\nsize(s4)=300,\nmapposition(vector2)={{x(float)=2,y(float)=2}}\n}},"),
        )
    };
    assert!(parse_map(&wh(1)).is_ok());
    assert!(parse_map(&wh(-1)).is_ok());
    assert!(parse_map(&wh(9)).unwrap_err().message.contains("not a valid object index"));
    assert!(parse_map(&wh(2)).unwrap_err().message.contains("not a wormhole"));
    assert!(parse_map(&wh(0)).unwrap_err().message.contains("itself"));
}

#[test]
fn terrain_data_length_checked() {
    let t = GOOD.replace(
        "objects(orderedarray<aumapobject>)=[2,",
        "objects(orderedarray<aumapobject>)=[3,minefieldterrain={\ndata(orderedarray<u1>)=[3,1,0,1],\nrows(s4)=2,\ncols(s4)=2,\ntilesize(s4)=500,\nmapposition(vector2)={x(float)=0,y(float)=0}\n},",
    );
    let e = err_of(&t);
    assert!(e.message.contains("rows*cols"), "{e}");
}

#[test]
fn index_errors() {
    assert!(parse_index("A|B|c|1|0\n").is_ok());
    let e = parse_index("A|B|c|1|0\nA|B|c|9|0\n").unwrap_err();
    assert_eq!(e.line, 2);
    let e = parse_index("A|B|c|1\n").unwrap_err();
    assert_eq!(e.line, 1);
    let e = parse_index("A|B|../c|1|0\n").unwrap_err();
    assert!(e.message.contains("stem"));
}

#[test]
fn garbage_does_not_panic() {
    for s in ["", "{", "}", "[", "{a=}", "{a(b=1}", "[x]", "{a(s4)=1,,}", "\u{feff}{}", "{a(s4)=--1}", &"{a=".repeat(500)] {
        let _ = parse_map(s);
    }
}
