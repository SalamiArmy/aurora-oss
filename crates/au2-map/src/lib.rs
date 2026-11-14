//! Loader and validator for the `.au2` level format described in `cleanroom/public/MAPFORMAT.md`.
//!
//! `parse_map` turns text into a typed [`Map`]; every error carries a line (and column where known).
//! Unknown fields are ignored; unknown object kinds are kept as [`Object::Unknown`] so object indices stay valid.

mod parse;

use parse::{Field, Kind, Parser, Value};
use std::fmt;
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------------------------------------ errors
#[derive(Debug, Clone, PartialEq)]
pub struct MapError {
    pub file: Option<String>,
    pub line: u32,
    pub col: u32,
    pub message: String,
}

impl MapError {
    pub fn at(line: u32, col: u32, message: impl Into<String>) -> Self {
        MapError { file: None, line, col, message: message.into() }
    }
    pub fn with_file(mut self, f: impl Into<String>) -> Self {
        self.file = Some(f.into());
        self
    }
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(file) = &self.file {
            write!(f, "{file}:")?;
        }
        write!(f, "line {}", self.line)?;
        if self.col > 0 {
            write!(f, ", col {}", self.col)?;
        }
        write!(f, ": {}", self.message)
    }
}
impl std::error::Error for MapError {}

// ------------------------------------------------------------------------------------------------ model
#[derive(Debug, Clone, PartialEq)]
pub struct FactionDef {
    pub id: String,
    pub think_period: u32,
    pub max_sources: u32,
    pub max_orders: u32,
}

pub type P2 = [f64; 2];

#[derive(Debug, Clone, PartialEq)]
pub struct SunDef {
    /// -1 = neutral, else faction index.
    pub faction: i32,
    pub health: i32,
    pub growth: i32,
    /// Starting level.
    pub power: i32,
    pub max_power: i32,
    pub starting_units: u32,
    pub awareness_radius: f64,
    pub pos: P2,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OrbitDef {
    pub rx: f64,
    pub ry: f64,
    pub rotation_deg: f64,
    pub initial_angle_deg: f64,
    pub speed: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FragileDef {
    pub sun: SunDef,
    pub orbit: OrbitDef,
    pub broken_seconds: u32,
    pub rebuild_frames: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WormholeDef {
    /// Index into `Map::objects`, or -1 for "random other wormhole".
    pub destination: i32,
    pub size: f64,
    pub orbit: OrbitDef,
    pub pos: P2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SupernovaDef {
    pub required_investment: u32,
    pub start_scale: f64,
    pub end_scale: f64,
    pub blast_radius: f64,
    pub collapse_radius: f64,
    pub kill_enemy_suns: bool,
    pub kill_enemy_units: bool,
    pub unit_residuals: u32,
    pub dormant_period: u32,
    pub pos: P2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhiteHoleDef {
    pub required_investment: u32,
    pub start_return: u32,
    pub end_return: u32,
    pub growth_rate: f64,
    pub game_size: f64,
    pub dormant_period: u32,
    pub pos: P2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GammaRayDef {
    pub kill_suns: bool,
    pub kill_units: bool,
    pub rotation_deg: f64,
    pub width: f64,
    pub period: u32,
    pub randomization: f64,
    pub random_start: bool,
    pub pos: P2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShockwaveDef {
    /// e.g. `st_incircle`; anything else is treated as a travelling line.
    pub kind: String,
    pub dormant: u32,
    pub period: u32,
    pub speed: f64,
    pub lifetime: u32,
    pub line_rotation_deg: f64,
    pub start_offset: f64,
    pub pos: P2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FogDef {
    pub sun_radius: f64,
    pub additional_per_ring: f64,
    pub unowned_radius: f64,
    pub enemy_radius: f64,
    pub non_sun_radius: f64,
    pub pos: P2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainKind {
    Force,
    Speed,
    Reflect,
    Minefield,
    Confusion,
    Neutral,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerrainDef {
    pub kind: TerrainKind,
    /// One 0/1 per tile, row-major (`rows * cols` entries). Rebuilt from `squares` if `data` is empty.
    pub data: Vec<u8>,
    pub tile_size: f64,
    pub rows: u32,
    pub cols: u32,
    pub outer_radius: f64,
    pub inner_radius: f64,
    pub switch_time: u32,
    pub enabled: bool,
    /// Grid bottom-left corner (or centre of a circular zone).
    pub pos: P2,
    pub acceleration: P2,
    pub dir_switch: i32,
    pub speed_change: f64,
    pub death_rate: f64,
    pub density: f64,
    pub faction_change_rate: f64,
    pub lose_order_rate: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Object {
    Sun(SunDef),
    OrbitingSun(SunDef, OrbitDef),
    FragileSun(FragileDef),
    Wormhole(WormholeDef),
    Supernova(SupernovaDef),
    WhiteHole(WhiteHoleDef),
    GammaRay(GammaRayDef),
    Shockwave(ShockwaveDef),
    Fog(FogDef),
    Terrain(TerrainDef),
    /// Unrecognised kind; kept so that indices of later objects stay correct.
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Map {
    pub factions: Vec<FactionDef>,
    pub objects: Vec<Object>,
    /// Source line of each object (parallel to `objects`), for diagnostics.
    pub object_lines: Vec<u32>,
}

// ------------------------------------------------------------------------------------------------ field access
struct Fields<'a> {
    fields: &'a [Field],
    line: u32,
    what: &'a str,
}

impl<'a> Fields<'a> {
    fn of(v: &'a Value, what: &'a str) -> Result<Fields<'a>, MapError> {
        match v.as_object() {
            Some(f) => Ok(Fields { fields: f, line: v.line, what }),
            None => Err(MapError::at(v.line, 0, format!("{what}: expected an object {{...}}"))),
        }
    }
    fn get(&self, name: &str) -> Option<&'a Field> {
        self.fields.iter().find(|f| f.name == name)
    }
    fn num(&self, name: &str, default: f64) -> Result<f64, MapError> {
        match self.get(name) {
            None => Ok(default),
            Some(f) => match f.value.kind {
                Kind::Num(n) => Ok(n),
                _ => Err(MapError::at(f.line, 0, format!("{}: field '{name}' must be a number", self.what))),
            },
        }
    }
    fn int(&self, name: &str, default: i64) -> Result<i64, MapError> {
        let n = self.num(name, default as f64)?;
        Ok(n.round() as i64)
    }
    fn uint(&self, name: &str, default: u32) -> Result<u32, MapError> {
        let n = self.int(name, default as i64)?;
        if n < 0 {
            let line = self.get(name).map(|f| f.line).unwrap_or(self.line);
            return Err(MapError::at(line, 0, format!("{}: field '{name}' must not be negative", self.what)));
        }
        Ok(n.min(u32::MAX as i64) as u32)
    }
    fn boolean(&self, name: &str, default: bool) -> Result<bool, MapError> {
        match self.get(name) {
            None => Ok(default),
            Some(f) => match f.value.kind {
                Kind::Bool(b) => Ok(b),
                Kind::Num(n) => Ok(n != 0.0),
                _ => Err(MapError::at(f.line, 0, format!("{}: field '{name}' must be true/false", self.what))),
            },
        }
    }
    fn word(&self, name: &str, default: &str) -> String {
        match self.get(name) {
            Some(Field { value: Value { kind: Kind::Ident(s), .. }, .. }) => s.clone(),
            _ => default.to_string(),
        }
    }
    fn vec2(&self, name: &str) -> Result<Option<P2>, MapError> {
        let Some(f) = self.get(name) else { return Ok(None) };
        let sub = Fields::of(&f.value, name)?;
        Ok(Some([sub.num("x", 0.0)?, sub.num("y", 0.0)?]))
    }
    fn position(&self) -> Result<P2, MapError> {
        match self.vec2("mapposition")? {
            Some(p) => Ok(p),
            None => Err(MapError::at(self.line, 0, format!("{}: missing required field 'mapposition'", self.what))),
        }
    }
    /// Items of an array-valued field (declared count is checked against the items present).
    fn array(&self, name: &str) -> Result<Option<(&'a [Value], u32)>, MapError> {
        let Some(f) = self.get(name) else { return Ok(None) };
        match &f.value.kind {
            Kind::Array { count, items } => {
                if *count != items.len() {
                    return Err(MapError::at(
                        f.line,
                        0,
                        format!("{}: array '{name}' declares {count} items but contains {}", self.what, items.len()),
                    ));
                }
                Ok(Some((items.as_slice(), f.line)))
            }
            _ => Err(MapError::at(f.line, 0, format!("{}: field '{name}' must be an array [n, ...]", self.what))),
        }
    }
}

fn orbit_of(f: &Fields, angle_key: &str) -> Result<OrbitDef, MapError> {
    Ok(OrbitDef {
        rx: f.num("orbitx", 0.0)?,
        ry: f.num("orbity", 0.0)?,
        rotation_deg: f.num("orbitrotation", 0.0)?,
        initial_angle_deg: f.num(angle_key, 0.0)?,
        speed: f.num("orbitspeed", 0.0)?,
    })
}

fn sun_of(f: &Fields) -> Result<SunDef, MapError> {
    Ok(SunDef {
        faction: f.int("faction", -1)? as i32,
        health: f.int("health", 100)? as i32,
        growth: f.int("growth", 0)? as i32,
        power: f.int("power", 0)? as i32,
        max_power: f.int("maxpower", 1)? as i32,
        starting_units: f.uint("startingunitcount", 0)?,
        awareness_radius: f.num("awarenessradius", -1.0)?,
        pos: f.position()?,
    })
}

fn terrain_of(kind: TerrainKind, f: &Fields) -> Result<TerrainDef, MapError> {
    let rows = f.uint("rows", 0)?;
    let cols = f.uint("cols", 0)?;
    let mut data = Vec::new();
    if let Some((items, line)) = f.array("data")? {
        for it in items {
            match it.as_f64() {
                Some(n) => data.push(if n != 0.0 { 1 } else { 0 }),
                None => return Err(MapError::at(it.line, 0, "terrain data entries must be 0/1 numbers")),
            }
        }
        if !data.is_empty() && data.len() as u64 != rows as u64 * cols as u64 {
            return Err(MapError::at(
                line,
                0,
                format!("{}: data has {} entries but rows*cols = {}*{} = {}", f.what, data.len(), rows, cols, rows as u64 * cols as u64),
            ));
        }
    }
    if data.is_empty() && rows > 0 && cols > 0 {
        // rebuild from the rectangle cover, if there is one
        data = vec![0; (rows * cols) as usize];
        if let Some((sq, _)) = f.array("squares")? {
            for s in sq {
                let sf = Fields::of(s, "terrain square")?;
                let (r, c, w, h) = (sf.uint("r", 0)?, sf.uint("c", 0)?, sf.uint("width", 1)?, sf.uint("height", 1)?);
                if r + h > rows || c + w > cols {
                    return Err(MapError::at(s.line, 0, "terrain square lies outside the grid"));
                }
                for rr in r..r + h {
                    for cc in c..c + w {
                        data[(rr * cols + cc) as usize] = 1;
                    }
                }
            }
        }
    }
    let tile_size = f.num("tilesize", 1000.0)?;
    if tile_size <= 0.0 {
        return Err(MapError::at(f.get("tilesize").map(|x| x.line).unwrap_or(f.line), 0, "tilesize must be positive"));
    }
    Ok(TerrainDef {
        kind,
        data,
        tile_size,
        rows,
        cols,
        outer_radius: f.num("outerradius", 0.0)?,
        inner_radius: f.num("innerradius", 0.0)?,
        switch_time: f.uint("switchtime", 0)?,
        enabled: f.boolean("enabled", true)?,
        pos: f.position()?,
        acceleration: f.vec2("acceleration")?.unwrap_or([0.0, 0.0]),
        dir_switch: f.int("dirswitch", 0)? as i32,
        speed_change: f.num("speedchange", 0.0)?,
        death_rate: f.num("deathrate", 0.0)?,
        density: f.num("density", 100.0)?,
        faction_change_rate: f.num("factionchangerate", 0.0)?,
        lose_order_rate: f.num("loseorderrate", 0.0)?,
    })
}

fn object_of(tag: &str, v: &Value) -> Result<Object, MapError> {
    let f = Fields::of(v, tag)?;
    Ok(match tag {
        "sun" => Object::Sun(sun_of(&f)?),
        "orbitingsun" => Object::OrbitingSun(sun_of(&f)?, orbit_of(&f, "initialangle")?),
        "fragilesun" => Object::FragileSun(FragileDef {
            sun: sun_of(&f)?,
            orbit: orbit_of(&f, "initialangle")?,
            broken_seconds: f.uint("brokenseconds", 240)?,
            rebuild_frames: f.uint("rebuildframes", 256)?,
        }),
        "wormhole" => Object::Wormhole(WormholeDef {
            destination: f.int("destinationwormhole", -1)? as i32,
            size: f.num("size", 400.0)?,
            orbit: orbit_of(&f, "initialorbitangle")?,
            pos: f.position()?,
        }),
        "supernova" => Object::Supernova(SupernovaDef {
            required_investment: f.uint("requiredinvestment", 400)?,
            start_scale: f.num("startscale", 400.0)?,
            end_scale: f.num("endscale", 600.0)?,
            blast_radius: f.num("blastradius", 7000.0)?,
            collapse_radius: f.num("collapseradius", 9000.0)?,
            kill_enemy_suns: f.boolean("killenemysuns", false)?,
            kill_enemy_units: f.boolean("killenemyunits", true)?,
            unit_residuals: f.uint("unitresiduals", 0)?,
            dormant_period: f.uint("dormantperiod", 180)?,
            pos: f.position()?,
        }),
        "whitehole" => Object::WhiteHole(WhiteHoleDef {
            required_investment: f.uint("requiredinvestment", 400)?,
            start_return: f.uint("startreturn", 600)?,
            end_return: f.uint("endreturn", 900)?,
            growth_rate: f.num("growthrate", 0.0)?,
            game_size: f.num("gamesize", 400.0)?,
            dormant_period: f.uint("dormantperiod", 600)?,
            pos: f.position()?,
        }),
        "gammaray" => Object::GammaRay(GammaRayDef {
            kill_suns: f.boolean("killsuns", true)?,
            kill_units: f.boolean("killunits", false)?,
            rotation_deg: f.num("rotation", 0.0)?,
            width: f.num("width", 800.0)?,
            period: f.uint("period", 60)?,
            randomization: f.num("randomization", 0.0)?,
            random_start: f.boolean("randomstarttime", false)?,
            pos: f.position()?,
        }),
        "shockwave" => Object::Shockwave(ShockwaveDef {
            kind: f.word("type", "st_incircle"),
            dormant: f.uint("dormant", 0)?,
            period: f.uint("period", 600)?,
            speed: f.num("speed", 10.0)?,
            lifetime: f.uint("lifetime", 600)?,
            line_rotation_deg: f.num("linerotation", 0.0)?,
            start_offset: f.num("startoffset", 0.0)?,
            pos: f.vec2("mapposition")?.unwrap_or([0.0, 0.0]),
        }),
        "fogofwar" => Object::Fog(FogDef {
            sun_radius: f.num("sunradius", 2500.0)?,
            additional_per_ring: f.num("additionalperring", 0.0)?,
            unowned_radius: f.num("unownedradius", 0.0)?,
            enemy_radius: f.num("enemyradius", 0.0)?,
            non_sun_radius: f.num("nonsunradius", 800.0)?,
            pos: f.vec2("mapposition")?.unwrap_or([0.0, 0.0]),
        }),
        "forceterrain" => Object::Terrain(terrain_of(TerrainKind::Force, &f)?),
        "speedterrain" => Object::Terrain(terrain_of(TerrainKind::Speed, &f)?),
        "reflectterrain" => Object::Terrain(terrain_of(TerrainKind::Reflect, &f)?),
        "minefieldterrain" => Object::Terrain(terrain_of(TerrainKind::Minefield, &f)?),
        "confusionterrain" => Object::Terrain(terrain_of(TerrainKind::Confusion, &f)?),
        "neutralterrain" => Object::Terrain(terrain_of(TerrainKind::Neutral, &f)?),
        other => Object::Unknown(other.to_string()),
    })
}

// ------------------------------------------------------------------------------------------------ top level
const FACTION_IDS: [&str; 4] = ["fi_player", "fi_player2", "fi_player3", "fi_player4"];

/// Parse and validate level text.
pub fn parse_map(text: &str) -> Result<Map, MapError> {
    let doc = Parser::new(text).parse_document()?;
    let top = Fields::of(&doc, "level")?;

    // factions
    let (fitems, fline) = top
        .array("factions")?
        .ok_or_else(|| MapError::at(doc.line, 0, "level: missing required field 'factions'"))?;
    if !(2..=4).contains(&fitems.len()) {
        return Err(MapError::at(fline, 0, format!("level needs 2 to 4 factions, found {}", fitems.len())));
    }
    let mut factions = Vec::new();
    for (i, item) in fitems.iter().enumerate() {
        let wrapper = Fields::of(item, "faction entry")?;
        // `factionobject(faction)=faction={...}`; accept the inner object directly as well
        let inner_val = wrapper.get("factionobject").map(|f| &f.value).unwrap_or(item);
        let ff = Fields::of(inner_val, "faction")?;
        let inner = match ff.get("faction") {
            Some(f) => Fields::of(&f.value, "faction")?,
            None => ff,
        };
        let id = inner.word("id", FACTION_IDS[i]);
        if !FACTION_IDS.contains(&id.as_str()) {
            return Err(MapError::at(item.line, 0, format!("unknown faction id '{id}'")));
        }
        if id != FACTION_IDS[i] {
            return Err(MapError::at(item.line, 0, format!("faction {i} must have id '{}', found '{id}'", FACTION_IDS[i])));
        }
        factions.push(FactionDef {
            id,
            think_period: inner.uint("thinkperiod", 0)?,
            max_sources: inner.uint("maxsources", 8)?,
            max_orders: inner.uint("maxorders", 8)?,
        });
    }

    // objects
    let (oitems, _) = top
        .array("objects")?
        .ok_or_else(|| MapError::at(doc.line, 0, "level: missing required field 'objects'"))?;
    let mut objects = Vec::new();
    let mut object_lines = Vec::new();
    for item in oitems {
        match &item.kind {
            Kind::Tagged(tag, inner) => {
                objects.push(object_of(tag, inner).map_err(|mut e| {
                    if e.line == 0 {
                        e.line = item.line;
                    }
                    e
                })?);
                object_lines.push(item.line);
            }
            _ => return Err(MapError::at(item.line, 0, "objects array entries must look like `kind={...}`")),
        }
    }
    let map = Map { factions, objects, object_lines };
    map.validate()?;
    Ok(map)
}

impl Map {
    /// Semantic checks (also run by `parse_map`).
    pub fn validate(&self) -> Result<(), MapError> {
        let nf = self.factions.len() as i32;
        let mut owned = vec![false; self.factions.len()];
        for (i, obj) in self.objects.iter().enumerate() {
            let line = self.object_lines.get(i).copied().unwrap_or(0);
            let sun = match obj {
                Object::Sun(s) | Object::OrbitingSun(s, _) => Some(s),
                Object::FragileSun(f) => Some(&f.sun),
                _ => None,
            };
            if let Some(s) = sun {
                if s.faction < -1 || s.faction >= nf {
                    return Err(MapError::at(line, 0, format!("object {i}: sun faction {} out of range (-1..{})", s.faction, nf - 1)));
                }
                if s.max_power < 1 {
                    return Err(MapError::at(line, 0, format!("object {i}: sun maxpower must be at least 1")));
                }
                if s.power < 0 || s.power > s.max_power {
                    return Err(MapError::at(line, 0, format!("object {i}: sun power {} not in 0..={}", s.power, s.max_power)));
                }
                if s.faction >= 0 {
                    owned[s.faction as usize] = true;
                }
            }
            if let Object::Wormhole(w) = obj {
                if w.size <= 0.0 {
                    return Err(MapError::at(line, 0, format!("object {i}: wormhole size must be positive")));
                }
                if w.destination != -1 {
                    let d = w.destination;
                    if d < 0 || d as usize >= self.objects.len() {
                        return Err(MapError::at(line, 0, format!("object {i}: wormhole destination {d} is not a valid object index")));
                    }
                    if d as usize == i {
                        return Err(MapError::at(line, 0, format!("object {i}: wormhole destination is itself")));
                    }
                    if !matches!(self.objects[d as usize], Object::Wormhole(_)) {
                        return Err(MapError::at(line, 0, format!("object {i}: wormhole destination {d} is not a wormhole")));
                    }
                }
            }
        }
        for (k, o) in owned.iter().enumerate() {
            if !o {
                return Err(MapError::at(1, 0, format!("faction {k} owns no sun")));
            }
        }
        let wormholes = self.objects.iter().filter(|o| matches!(o, Object::Wormhole(_))).count();
        if wormholes == 1 {
            let i = self.objects.iter().position(|o| matches!(o, Object::Wormhole(_))).unwrap();
            if matches!(&self.objects[i], Object::Wormhole(w) if w.destination == -1) {
                return Err(MapError::at(self.object_lines[i], 0, "a lone wormhole with a random destination has nowhere to go"));
            }
        }
        Ok(())
    }
}

// ------------------------------------------------------------------------------------------------ level index
#[derive(Debug, Clone, PartialEq)]
pub struct LevelEntry {
    pub constellation: String,
    pub name: String,
    pub stem: String,
    pub difficulty: u8,
    pub bonus: bool,
}

/// Parse `levels.txt` (`Constellation|Level name|file stem|difficulty 0-3|bonus 0/1`).
pub fn parse_index(text: &str) -> Result<Vec<LevelEntry>, MapError> {
    let mut out = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = n as u32 + 1;
        let l = raw.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = l.split('|').map(str::trim).collect();
        if parts.len() != 5 {
            return Err(MapError::at(line, 0, format!("expected 5 '|'-separated fields, found {}", parts.len())));
        }
        let difficulty: u8 = parts[3]
            .parse()
            .ok()
            .filter(|d| *d <= 3)
            .ok_or_else(|| MapError::at(line, 0, format!("difficulty must be 0..3, found '{}'", parts[3])))?;
        let bonus = match parts[4] {
            "0" => false,
            "1" => true,
            x => return Err(MapError::at(line, 0, format!("bonus must be 0 or 1, found '{x}'"))),
        };
        if parts[2].is_empty() || parts[2].contains(['/', '\\', '.']) {
            return Err(MapError::at(line, 0, format!("invalid file stem '{}'", parts[2])));
        }
        out.push(LevelEntry {
            constellation: parts[0].to_string(),
            name: parts[1].to_string(),
            stem: parts[2].to_string(),
            difficulty,
            bonus,
        });
    }
    Ok(out)
}

/// Load `<dir>/levels.txt`.
pub fn load_index(dir: &Path) -> Result<Vec<LevelEntry>, MapError> {
    let p = dir.join("levels.txt");
    let text = std::fs::read_to_string(&p).map_err(|e| MapError::at(0, 0, format!("cannot read file: {e}")).with_file(p.display().to_string()))?;
    parse_index(&text).map_err(|e| e.with_file(p.display().to_string()))
}

pub fn level_path(dir: &Path, entry: &LevelEntry) -> PathBuf {
    dir.join(format!("{}.au2", entry.stem))
}

/// Load one level file; errors carry the file name.
pub fn load_map(path: &Path) -> Result<Map, MapError> {
    let name = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|e| MapError::at(0, 0, format!("cannot read file: {e}")).with_file(name.clone()))?;
    parse_map(&text).map_err(|e| e.with_file(name))
}

pub fn load_level(dir: &Path, entry: &LevelEntry) -> Result<Map, MapError> {
    load_map(&level_path(dir, entry))
}
