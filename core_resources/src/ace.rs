//! ACE (Access Control Entries) permission system.
//!
//! Funguje stejně jako FiveM/CitizenFX:
//!
//! ```text
//! AddPrincipal("identifier.ip:1.2.3.4", "group.admin")
//! AddPrincipal("group.admin", "group.moderator")   -- dědičnost
//! AddAce("group.admin", "command.*",     "allow")
//! AddAce("group.moderator", "command.kick", "allow")
//! AddAce("group.admin", "command.rcon",  "deny")   -- deny přebíjí allow
//! ```
//!
//! Principals hráče jsou sestaveny z:
//! * `player:{player_id}` (runtime ID)
//! * `identifier.{type}:{value}` pro každý uložený identifikátor
//! * `everyone` — každý hráč automaticky
//!
//! Wildcard v ACE: `command.*` odpovídá libovolnému `command.xxx`.
//! `deny` přebíjí `allow` kdekoliv v dědičném řetězci.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;

// ---------------------------------------------------------------------------
// AceRegistryInner
// ---------------------------------------------------------------------------

#[derive(Default)]
struct AceRegistryInner {
    /// principal → {parent_principal, ...}  (dědičnost)
    inheritance: HashMap<String, HashSet<String>>,
    /// principal → ace_name → allow(true)/deny(false)
    aces: HashMap<String, HashMap<String, bool>>,
    /// player_id → vec identifikátorů (např. "ip:1.2.3.4", "discord:123")
    identifiers: HashMap<u64, Vec<String>>,
}

impl AceRegistryInner {
    fn add_ace(&mut self, principal: &str, ace: &str, allow: bool) {
        self.aces
            .entry(principal.to_string())
            .or_default()
            .insert(ace.to_string(), allow);
    }

    fn remove_ace(&mut self, principal: &str, ace: &str) {
        if let Some(map) = self.aces.get_mut(principal) {
            map.remove(ace);
        }
    }

    fn add_principal(&mut self, child: &str, parent: &str) {
        self.inheritance
            .entry(child.to_string())
            .or_default()
            .insert(parent.to_string());
    }

    fn remove_principal(&mut self, child: &str, parent: &str) {
        if let Some(set) = self.inheritance.get_mut(child) {
            set.remove(parent);
        }
    }

    fn add_identifier(&mut self, player_id: u64, identifier: String) {
        let ids = self.identifiers.entry(player_id).or_default();
        if !ids.contains(&identifier) {
            ids.push(identifier);
        }
    }

    fn remove_player(&mut self, player_id: u64) {
        self.identifiers.remove(&player_id);
        // Odstraň i případné player:{id} záznamy v inheritance/aces
        let key = format!("player:{player_id}");
        self.inheritance.remove(&key);
        self.aces.remove(&key);
    }

    fn get_identifiers(&self, player_id: u64) -> Vec<String> {
        self.identifiers
            .get(&player_id)
            .cloned()
            .unwrap_or_default()
    }

    fn get_identifier(&self, player_id: u64, id_type: &str) -> Option<String> {
        let prefix = format!("{id_type}:");
        self.identifiers.get(&player_id)?.iter()
            .find(|s| s.starts_with(&prefix))
            .cloned()
    }

    /// Rekurzivně sbírá výsledky ACE pro daný principal (s dědičností).
    /// `visited` zabraňuje cyklům.
    fn collect(&self, principal: &str, ace: &str, visited: &mut HashSet<String>, out: &mut Vec<bool>) {
        // Přímý záznam pro tento principal
        if let Some(map) = self.aces.get(principal) {
            // 1. Exact match
            if let Some(&v) = map.get(ace) {
                out.push(v);
            }
            // 2. Namespace wildcard: "command.noclip" → check "command.*"
            if let Some(dot) = ace.find('.') {
                let wildcard = format!("{}.*", &ace[..dot]);
                if let Some(&v) = map.get(&wildcard) {
                    out.push(v);
                }
            }
            // 3. Global wildcard
            if let Some(&v) = map.get("*") {
                out.push(v);
            }
        }
        // Walk inheritance
        if let Some(parents) = self.inheritance.get(principal) {
            for parent in parents.clone() {
                if visited.insert(parent.clone()) {
                    self.collect(&parent, ace, visited, out);
                }
            }
        }
    }

    /// Zkontroluje, zda `principal` má povolení k `ace`.
    /// deny přebíjí allow kdekoliv v řetězci.
    fn is_allowed(&self, principal: &str, ace: &str) -> bool {
        let mut visited = HashSet::from([principal.to_string()]);
        let mut results = Vec::new();
        self.collect(principal, ace, &mut visited, &mut results);
        if results.iter().any(|&r| !r) {
            return false;
        }
        results.iter().any(|&r| r)
    }

    /// Zkontroluje, zda hráč (identifikovaný player_id) má povolení.
    /// Prochází `player:{id}`, `identifier.*` i `everyone`.
    fn is_player_allowed(&self, player_id: u64, ace: &str) -> bool {
        let mut visited: HashSet<String> = HashSet::new();
        let mut results: Vec<bool> = Vec::new();

        // player:{id}
        let player_principal = format!("player:{player_id}");
        if visited.insert(player_principal.clone()) {
            self.collect(&player_principal, ace, &mut visited, &mut results);
        }

        // identifier.*
        if let Some(ids) = self.identifiers.get(&player_id) {
            for id in ids.clone() {
                let p = format!("identifier.{id}");
                if visited.insert(p.clone()) {
                    self.collect(&p, ace, &mut visited, &mut results);
                }
            }
        }

        // everyone (implicitní skupina každého hráče)
        if visited.insert("everyone".to_string()) {
            self.collect("everyone", ace, &mut visited, &mut results);
        }

        if results.iter().any(|&r| !r) {
            return false;
        }
        results.iter().any(|&r| r)
    }
}

// ---------------------------------------------------------------------------
// Public AceRegistry (Arc wrapper — Bevy Resource + clone do sandboxů)
// ---------------------------------------------------------------------------

#[derive(Resource, Clone, Default)]
pub struct AceRegistry(Arc<Mutex<AceRegistryInner>>);

impl AceRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, AceRegistryInner> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn add_ace(&self, principal: &str, ace: &str, allow: bool) {
        self.lock().add_ace(principal, ace, allow);
    }

    pub fn remove_ace(&self, principal: &str, ace: &str) {
        self.lock().remove_ace(principal, ace);
    }

    pub fn add_principal(&self, child: &str, parent: &str) {
        self.lock().add_principal(child, parent);
    }

    pub fn remove_principal(&self, child: &str, parent: &str) {
        self.lock().remove_principal(child, parent);
    }

    pub fn add_identifier(&self, player_id: u64, identifier: impl Into<String>) {
        self.lock().add_identifier(player_id, identifier.into());
    }

    pub fn remove_player(&self, player_id: u64) {
        self.lock().remove_player(player_id);
    }

    pub fn get_identifiers(&self, player_id: u64) -> Vec<String> {
        self.lock().get_identifiers(player_id)
    }

    pub fn get_identifier(&self, player_id: u64, id_type: &str) -> Option<String> {
        self.lock().get_identifier(player_id, id_type)
    }

    pub fn is_allowed(&self, principal: &str, ace: &str) -> bool {
        self.lock().is_allowed(principal, ace)
    }

    pub fn is_player_allowed(&self, player_id: u64, ace: &str) -> bool {
        self.lock().is_player_allowed(player_id, ace)
    }
}
