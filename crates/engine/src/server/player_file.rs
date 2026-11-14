//! A player coming and going: its file, joining, the mode it left in.

use super::*;
use weft_abi::entry_kind as ek;

impl Server {
    pub(super) fn join(&mut self, c: &'static Content, pi: usize, name: String, key: String) {
        let key = if key.is_empty() { name.clone() } else { key };
        if self.players.iter().any(|p| p.joined && p.key == key) {
            self.players[pi]
                .conn
                .send(ToClient::Disconnect(format!("{name} is already playing")));
            return;
        }
        // Read each time, so a key added takes effect with no restart. The key is what the
        // connection proved, not what the client claims.
        if let Some(Ok(list)) = self.cfg.whitelist.as_ref().map(std::fs::read_to_string) {
            if !list.lines().any(|l| l.trim() == key) {
                log::info!("{name} is not on the whitelist; to let them in, add this line: {key}");
                self.players[pi]
                    .conn
                    .send(ToClient::Disconnect("not on this server's whitelist".into()));
                return;
            }
        }
        let saved = self
            .storage
            .as_ref()
            .and_then(|s| load_player(c, &s.lock(), &key, |dim| self.dims[dim as usize].time));
        let spawn = || (c.spawn_dim, self.dims[c.spawn_dim as usize].spawn_point(c));
        let PlayerData { dim, mut entity, inventory, components, deaths } = saved.unwrap_or_else(|| {
            let (dim, pos) = spawn();
            let mut entity = Entity::new(0, c.game.player_kind, pos, -1.0);
            entity.mode = self
                .storage
                .as_ref()
                .map_or(GameMode::SURVIVAL, |s| s.lock().meta.game_mode);
            PlayerData { dim: Some(dim), entity, inventory: None, components: Vec::new(), deaths: 0 }
        });
        let dim = match dim {
            Some(d) if d != c.void_dim || c.spawn_dim == c.void_dim => d,
            _ => {
                let (d, pos) = spawn();
                entity.pos = pos;
                log::warn!("player {name} was in a dimension that is not there now, placed at the spawn");
                d
            }
        };
        let pos = entity.pos;
        let r = self.dims[dim as usize].ensure_region(c, pos);
        let id = r.new_id();
        let mut e = entity;
        e.id = id;
        e.kind = c.game.player_kind;
        e.player = true;
        let look = [e.yaw, e.pitch];
        let flying = e.flying;
        // The first connection is the one playing on this machine, the world's owner, with
        // commands if the world was made with cheats.
        let cheats = self.storage.as_ref().is_none_or(|s| s.lock().meta.cheats);
        let permission = if pi == 0 && !self.cfg.dedicated && cheats {
            abi::permission::OWNER
        } else {
            self.storage
                .as_ref()
                .and_then(|s| s.lock().meta.ops.iter().find(|o| o.0 == key).map(|o| o.1))
                .unwrap_or(abi::permission::EVERYONE)
        };
        e.permission = permission;
        e.selected = e
            .selected
            .min(c.game.player_layout.hotbar.saturating_sub(1));
        let selected = e.selected;
        r.entities.insert_with(c, e, inventory, components);
        r.keep_sources(c, id);
        // Known only now that the modifiers are back; a new player (no saved health) starts full.
        let max = r.max_health(c, id);
        let mut dead = false;
        if let Some(e) = r.entities.get_mut(id) {
            if e.health < 0.0 {
                e.health = max;
            }
            dead = !e.alive();
        }
        r.emit_engine(c, abi::event::PLAYER_JOINED, &abi::PlayerEvent { player: id });
        let p = &mut self.players[pi];
        p.name = name;
        p.key = key;
        p.joined = true;
        p.dim = dim;
        p.entity = id;
        p.last_place = (None, pos);
        p.deaths = deaths;
        p.first_sync = true;
        p.glow_sent = false;
        log::info!("{} joined", p.name);
        let line = Msg::new("multiplayer.joined", vec![Arg::Text(p.name.clone())]);
        for q in self.players.iter_mut().filter(|q| q.joined) {
            q.conn
                .send(ToClient::Show(crate::net::Shown::chat(line.clone())));
        }
        let p = &mut self.players[pi];
        p.conn
            .send(ToClient::Welcome { player: id, dim, pos, seed: self.cfg.seed, look, selected, flying });
        self.send_lod_columns(pi, dim);
        let p = &mut self.players[pi];
        p.conn.send(ToClient::Edits {
            dim,
            specs: self.dims[dim as usize]
                .edits
                .iter()
                .map(|e| e.spec(c))
                .collect(),
            all: true,
        });
        p.conn.send(ToClient::Configs(crate::config::synced()));
        // Left while dead: still dead, as in vanilla.
        if dead {
            p.conn.send(ToClient::Died);
        }
        if dim == c.void_dim {
            p.conn
                .send(ToClient::Show(crate::net::Shown::chat(void_note(c))));
        }
        // The skins of who is here already.
        let skins: Vec<(u64, Arc<Vec<u8>>)> = self
            .players
            .iter()
            .filter(|o| o.joined && o.entity != id)
            .filter_map(|o| o.skin.clone().map(|s| (o.entity, s)))
            .collect();
        for (entity, png) in skins {
            self.players[pi]
                .conn
                .send(ToClient::Skin { entity, png: png.to_vec() });
        }
        let avatars: Vec<(u64, Arc<crate::net::AvatarData>)> = self
            .players
            .iter()
            .filter(|o| o.joined && o.entity != id)
            .filter_map(|o| o.avatar.clone().map(|a| (o.entity, a)))
            .collect();
        for (entity, avatar) in avatars {
            self.players[pi]
                .conn
                .send(ToClient::Avatar { entity, avatar: (*avatar).clone() });
        }
    }
}

pub(super) fn save_player(s: &Storage, p: &Player, d: &Dim) {
    let c = content();
    let path = s.player_path(&p.key);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Some(e) = d.entities.get(p.entity) else { return };
    let comps = d
        .entities
        .comps
        .iter()
        .filter(|((id, _), _)| *id == p.entity)
        .map(|((_, comp), bytes)| (*comp, bytes.as_ref()))
        .collect();
    let mut bytes = Vec::new();
    super::persist::write(c, &mut bytes, &[(e, d.entities.inventories.get(&p.entity), comps, None)], d.time);
    let text =
        format!("name = {}\ndim = {}\ndeaths = {}\nentity = {}\n", p.name, c.dims[p.dim as usize].name, p.deaths, crate::kv::hex(&bytes));
    // Whole or not at all: a crash in the middle must not leave half a player.
    let tmp = path.with_extension("new");
    if let Err(err) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)) {
        log::error!("saving player {}: {err}", p.name);
    }
}

pub(super) struct PlayerData {
    pub dim: Option<u32>,
    pub entity: Entity,
    pub inventory: Option<Vec<ItemStack>>,
    pub components: Vec<(u32, Box<[u8]>)>,
    pub deaths: u32,
}

/// Account metadata wraps the same entity record as cube saves.
pub(super) fn load_player(c: &Content, s: &Storage, key: &str, time: impl Fn(u32) -> u64) -> Option<PlayerData> {
    let path = s.player_path(key);
    let src = match std::fs::read_to_string(&path) {
        Ok(src) => src,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            log::error!("reading player {key}: {e}; kept as .bak, the player starts anew");
            let _ = std::fs::copy(&path, path.with_extension("bak"));
            return None;
        }
    };
    let dim = crate::kv::get(&src, "dim").and_then(|name| c.game.catalog.find(ek::DIMENSION, name));
    let destination = dim
        .filter(|&d| d != c.void_dim || c.spawn_dim == c.void_dim)
        .unwrap_or(c.spawn_dim);
    let now = time(destination);
    let bytes = crate::kv::unhex(crate::kv::get(&src, "entity").unwrap_or_default());
    let Some((entity, inventory, components, _)) = super::persist::read(c, &bytes, now).pop() else {
        log::error!("player {key}: invalid entity record; kept as .bak, the player starts anew");
        let _ = std::fs::copy(&path, path.with_extension("bak"));
        return None;
    };
    let deaths = crate::kv::get(&src, "deaths")
        .and_then(|d| d.parse().ok())
        .unwrap_or(0);
    Some(PlayerData { dim, entity, inventory, components, deaths })
}
