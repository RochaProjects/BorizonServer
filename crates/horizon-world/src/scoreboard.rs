//! Vanilla scoreboard state, gameplay rules, ordering and `scoreboard.dat` NBT codec.
//!
//! This is the server-owned data/behavior subset of the reference scoreboard:
//! components remain JSON values and this module has no translation, Text,
//! rendering, client registry aggregate, or UI dependency.
use crate::chat_component::{
    Component, canonical_component, canonical_style, component_tag, json_to_tag,
    normalize_hover_entity_uuids, tag_to_json,
};
use crate::nbt::Tag;
use horizon_core::fastutil_order::{HashSetOrder, OpenOrder};
use horizon_core::{scoreboard_slots, team_color_names};
use serde_json::Value;
use std::collections::BTreeMap;

pub use horizon_core::scoreboard_criteria::{
    CRITERIA_AIR, CRITERIA_ARMOR, CRITERIA_DEATH_COUNT, CRITERIA_DUMMY, CRITERIA_FOOD,
    CRITERIA_HEALTH, CRITERIA_LEVEL, CRITERIA_PLAYER_KILLS, CRITERIA_TOTAL_KILLS, CRITERIA_TRIGGER,
    CRITERIA_XP, CUSTOM_CRITERIA, CUSTOM_STATS, STAT_TYPES,
};
pub use horizon_core::scoreboard_slots::{SLOT_BELOW_NAME, SLOT_LIST, SLOT_SIDEBAR};

/// A literal text component in JSON form.
pub fn literal(text: &str) -> Component {
    Value::String(text.to_owned())
}

/// `NumberFormat`: the representation used to display an objective score.
#[derive(Clone, Debug, PartialEq)]
pub enum NumberFormat {
    Blank,
    Fixed(Component),
    /// A component style object, for example `{ "color": "red", "bold": true }`.
    Styled(Component),
}
impl NumberFormat {
    pub fn sidebar_default() -> Self {
        NumberFormat::Styled(serde_json::json!({"color": "red"}))
    }
    pub fn player_list_default() -> Self {
        NumberFormat::Styled(serde_json::json!({"color": "yellow"}))
    }
    /// Vanilla `NumberFormatTypes.CODEC` encoding.
    pub fn to_json(&self) -> Value {
        match self {
            NumberFormat::Blank => serde_json::json!({"type": "minecraft:blank"}),
            NumberFormat::Fixed(component) => serde_json::json!({
                "type": "minecraft:fixed",
                "value": canonical_component(component)
            }),
            NumberFormat::Styled(style) => {
                let mut value = canonical_style(style);
                if let Value::Object(object) = &mut value {
                    object.insert("type".into(), Value::String("minecraft:styled".into()));
                }
                value
            }
        }
    }
    /// Vanilla `NumberFormatTypes.CODEC` decoding.
    pub fn from_json(value: &Value) -> Option<Self> {
        let kind = value.get("type")?.as_str()?;
        match kind.trim_start_matches("minecraft:") {
            "blank" => Some(NumberFormat::Blank),
            "fixed" => Some(NumberFormat::Fixed(value.get("value")?.clone())),
            "styled" => {
                let mut style = value.clone();
                if let Value::Object(object) = &mut style {
                    object.remove("type");
                }
                Some(NumberFormat::Styled(style))
            }
            _ => None,
        }
    }
    pub fn to_tag(&self) -> Tag {
        let mut tag = json_to_tag(&self.to_json());
        normalize_hover_entity_uuids(&mut tag);
        tag
    }
    pub fn from_tag(tag: &Tag) -> Option<Self> {
        Self::from_json(&tag_to_json(tag))
    }
}

/// The vanilla objective score render type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderType {
    Integer,
    Hearts,
}
impl RenderType {
    pub fn id(self) -> &'static str {
        match self {
            RenderType::Integer => "integer",
            RenderType::Hearts => "hearts",
        }
    }
    pub fn by_id(id: &str) -> Self {
        if id == "hearts" {
            RenderType::Hearts
        } else {
            RenderType::Integer
        }
    }
}

/// Vanilla `DisplaySlot` names, in registry/id order.
pub fn display_slots() -> &'static [&'static str] {
    &scoreboard_slots::DISPLAY_SLOTS
}

/// Whether a name is one of the vanilla scoreboard display slots.
pub fn display_slot_valid(name: &str) -> bool {
    scoreboard_slots::display_slot_valid(name)
}

/// Team name-tag/death-message visibility options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Always,
    Never,
    HideForOtherTeams,
    HideForOwnTeam,
}
impl Visibility {
    pub const ALL: [Visibility; 4] = [
        Visibility::Always,
        Visibility::Never,
        Visibility::HideForOtherTeams,
        Visibility::HideForOwnTeam,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Visibility::Always => "always",
            Visibility::Never => "never",
            Visibility::HideForOtherTeams => "hideForOtherTeams",
            Visibility::HideForOwnTeam => "hideForOwnTeam",
        }
    }
    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|visibility| visibility.name() == name)
    }
}

/// Team collision options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollisionRule {
    Always,
    Never,
    PushOtherTeams,
    PushOwnTeam,
}
impl CollisionRule {
    pub const ALL: [CollisionRule; 4] = [
        CollisionRule::Always,
        CollisionRule::Never,
        CollisionRule::PushOtherTeams,
        CollisionRule::PushOwnTeam,
    ];
    pub fn name(self) -> &'static str {
        match self {
            CollisionRule::Always => "always",
            CollisionRule::Never => "never",
            CollisionRule::PushOtherTeams => "pushOtherTeams",
            CollisionRule::PushOwnTeam => "pushOwnTeam",
        }
    }
    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|rule| rule.name() == name)
    }
}

/// A vanilla player team and its members.
#[derive(Clone, Debug, PartialEq)]
pub struct Team {
    pub name: String,
    pub display_name: Component,
    pub color: Option<String>,
    pub allow_friendly_fire: bool,
    pub see_friendly_invisibles: bool,
    pub prefix: Component,
    pub suffix: Component,
    pub nametag_visibility: Visibility,
    pub death_message_visibility: Visibility,
    pub collision_rule: CollisionRule,
    /// Member names in vanilla `HashSet` iteration order.
    pub players: HashSetOrder,
}
impl Team {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            display_name: literal(name),
            color: None,
            allow_friendly_fire: true,
            see_friendly_invisibles: true,
            prefix: literal(""),
            suffix: literal(""),
            nametag_visibility: Visibility::Always,
            death_message_visibility: Visibility::Always,
            collision_rule: CollisionRule::Always,
            players: HashSetOrder::default(),
        }
    }
    /// `PlayerTeam.getFormattedName` as a JSON component, without resolving presentation.
    pub fn formatted_name_component(&self, member: &Component) -> Component {
        let mut component = serde_json::json!({
            "text": "",
            "extra": [self.prefix.clone(), member.clone(), self.suffix.clone()]
        });
        if let Some(color) = &self.color {
            component["color"] = Value::String(color.clone());
        }
        component
    }
}

/// `Team.isAlliedTo`: both entities are members of the same team.
pub fn teams_allied(own: Option<&Team>, other: Option<&Team>) -> bool {
    matches!((own, other), (Some(a), Some(b)) if a.name == b.name)
}

/// `Entity.doTeamsAllowDamage` / `Player.canHarmPlayer`.
pub fn teams_allow_damage(own: Option<&Team>, other: Option<&Team>) -> bool {
    match own {
        None => true,
        Some(team) => !teams_allied(own, other) || team.allow_friendly_fire,
    }
}

/// Server-side `EntitySelector.pushableBy(entity).test(input)` team rule.
pub fn pushable_by(own: Option<&Team>, their: Option<&Team>, their_pushable: bool) -> bool {
    let own_rule = own.map_or(CollisionRule::Always, |team| team.collision_rule);
    if own_rule == CollisionRule::Never || !their_pushable {
        return false;
    }
    let their_rule = their.map_or(CollisionRule::Always, |team| team.collision_rule);
    if their_rule == CollisionRule::Never {
        return false;
    }
    let same = teams_allied(own, their);
    if (own_rule == CollisionRule::PushOwnTeam || their_rule == CollisionRule::PushOwnTeam) && same
    {
        return false;
    }
    own_rule != CollisionRule::PushOtherTeams && their_rule != CollisionRule::PushOtherTeams || same
}

/// `Entity.isInvisibleTo(player)` team rule.
pub fn invisible_to(
    own: Option<&Team>,
    viewer: Option<&Team>,
    viewer_spectator: bool,
    invisible: bool,
) -> bool {
    if viewer_spectator {
        return false;
    }
    match own {
        Some(team) if teams_allied(own, viewer) && team.see_friendly_invisibles => false,
        _ => invisible,
    }
}

/// `ServerPlayer.die`: whether `other` receives the death message for `dying`.
pub fn death_message_reaches(board: &Scoreboard, dying: &str, other: &str) -> bool {
    let Some(team) = board.players_team(dying) else {
        return true;
    };
    let same_team = board
        .players_team(other)
        .is_some_and(|other_team| other_team.name == team.name);
    match team.death_message_visibility {
        Visibility::Always => true,
        Visibility::Never => false,
        Visibility::HideForOtherTeams => same_team && other != dying,
        Visibility::HideForOwnTeam => !same_team,
    }
}

/// A scoreboard objective.
#[derive(Clone, Debug, PartialEq)]
pub struct Objective {
    pub name: String,
    pub criteria: String,
    pub display_name: Component,
    pub render_type: RenderType,
    pub display_auto_update: bool,
    pub number_format: Option<NumberFormat>,
}

/// One score value and its optional component/format overrides.
#[derive(Clone, Debug, PartialEq)]
pub struct Score {
    pub value: i32,
    pub locked: bool,
    pub display: Option<Component>,
    pub number_format: Option<NumberFormat>,
}
impl Default for Score {
    fn default() -> Self {
        Self {
            value: 0,
            locked: true,
            display: None,
            number_format: None,
        }
    }
}

/// One row returned by `listPlayerScores(objective)`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreEntry {
    pub owner: String,
    pub value: i32,
    pub display: Option<Component>,
    pub number_format: Option<NumberFormat>,
}
impl ScoreEntry {
    pub fn is_hidden(&self) -> bool {
        self.owner.starts_with('#')
    }
}

/// Vanilla scoreboard state with observable Java collection iteration order.
#[derive(Clone, Debug, PartialEq)]
pub struct Scoreboard {
    /// Objectives in `objectivesByName` iterator order.
    pub objectives: Vec<Objective>,
    objective_order: OpenOrder,
    by_criteria: BTreeMap<String, Vec<String>>,
    /// Scores by holder and objective. Empty holders remain tracked after objective removal.
    pub scores: BTreeMap<String, BTreeMap<String, Score>>,
    holder_order: OpenOrder,
    pub display: BTreeMap<String, String>,
    /// Teams in `teamsByName` iterator order.
    pub teams: Vec<Team>,
    team_order: OpenOrder,
    team_of: BTreeMap<String, String>,
}
impl Default for Scoreboard {
    fn default() -> Self {
        Self {
            objectives: Vec::new(),
            objective_order: OpenOrder::new(16, 0.5),
            by_criteria: BTreeMap::new(),
            scores: BTreeMap::new(),
            holder_order: OpenOrder::new(16, 0.5),
            display: BTreeMap::new(),
            teams: Vec::new(),
            team_order: OpenOrder::default(),
            team_of: BTreeMap::new(),
        }
    }
}

fn ranked<T>(items: &mut [T], order: &OpenOrder, name: impl Fn(&T) -> &str) {
    let rank: BTreeMap<&str, usize> = order
        .iter()
        .enumerate()
        .map(|(index, key)| (key, index))
        .collect();
    items.sort_by_key(|item| rank.get(name(item)).copied().unwrap_or(usize::MAX));
}

impl Scoreboard {
    fn reorder(&mut self) {
        ranked(&mut self.objectives, &self.objective_order, |objective| {
            &objective.name
        });
        ranked(&mut self.teams, &self.team_order, |team| &team.name);
    }
    pub fn objective(&self, name: &str) -> Option<&Objective> {
        self.objectives
            .iter()
            .find(|objective| objective.name == name)
    }
    pub fn objective_mut(&mut self, name: &str) -> Option<&mut Objective> {
        self.objectives
            .iter_mut()
            .find(|objective| objective.name == name)
    }
    /// Objectives in the order used by vanilla stream-based operations and saving.
    pub fn objectives_streamed(&self) -> Vec<&Objective> {
        self.objective_order
            .stream()
            .filter_map(|name| self.objective(name))
            .collect()
    }
    pub fn add_objective(
        &mut self,
        name: &str,
        criteria: &str,
        display_name: Component,
        render_type: RenderType,
        display_auto_update: bool,
        number_format: Option<NumberFormat>,
    ) -> bool {
        if self.objective(name).is_some() {
            return false;
        }
        self.objectives.push(Objective {
            name: name.to_owned(),
            criteria: criteria.to_owned(),
            display_name,
            render_type,
            display_auto_update,
            number_format,
        });
        self.objective_order.insert(name);
        self.by_criteria
            .entry(criteria.to_owned())
            .or_default()
            .push(name.to_owned());
        self.reorder();
        true
    }
    /// Removing an objective leaves score holders tracked even if their score maps become empty.
    pub fn remove_objective(&mut self, name: &str) {
        let Some(criteria) = self
            .objective(name)
            .map(|objective| objective.criteria.clone())
        else {
            return;
        };
        self.objectives.retain(|objective| objective.name != name);
        self.objective_order.remove(name);
        self.reorder();
        self.display.retain(|_, objective| objective != name);
        if let Some(list) = self.by_criteria.get_mut(&criteria) {
            list.retain(|objective| objective != name);
        }
        for scores in self.scores.values_mut() {
            scores.remove(name);
        }
    }
    pub fn display_objective(&self, slot: &str) -> Option<&Objective> {
        self.objective(self.display.get(slot)?)
    }
    pub fn set_display_objective(&mut self, slot: &str, objective: Option<&str>) {
        match objective {
            Some(objective) => {
                self.display.insert(slot.to_owned(), objective.to_owned());
            }
            None => {
                self.display.remove(slot);
            }
        }
    }
    pub fn score(&self, holder: &str, objective: &str) -> Option<&Score> {
        self.scores.get(holder)?.get(objective)
    }
    /// New vanilla scores start locked and at zero.
    pub fn score_mut(&mut self, holder: &str, objective: &str) -> &mut Score {
        self.holder_order.insert(holder);
        self.scores
            .entry(holder.to_owned())
            .or_default()
            .entry(objective.to_owned())
            .or_default()
    }
    pub fn get(&mut self, holder: &str, objective: &str) -> i32 {
        self.score_mut(holder, objective).value
    }
    /// Set a score, applying `display_auto_update` to the holder component when supplied.
    pub fn set_with(
        &mut self,
        holder: &str,
        objective: &str,
        value: i32,
        display: Option<&Component>,
    ) {
        let auto = self
            .objective(objective)
            .is_some_and(|objective| objective.display_auto_update);
        let score = self.score_mut(holder, objective);
        if auto
            && let Some(display) = display
            && score.display.as_ref().map(canonical_component) != Some(canonical_component(display))
        {
            score.display = Some(display.clone());
        }
        score.value = value;
    }
    pub fn set(&mut self, holder: &str, objective: &str, value: i32) {
        self.set_with(holder, objective, value, None);
    }
    pub fn add(&mut self, holder: &str, objective: &str, amount: i32) -> i32 {
        let value = self.get(holder, objective).wrapping_add(amount);
        self.set(holder, objective, value);
        value
    }
    /// Change a holder's score in every objective using this criteria, in objective creation order.
    pub fn for_all_objectives(
        &mut self,
        criteria: &str,
        holder: &str,
        update: impl FnMut(i32) -> i32,
    ) {
        self.for_all_objectives_with(criteria, holder, None, update);
    }
    pub fn for_all_objectives_with(
        &mut self,
        criteria: &str,
        holder: &str,
        display: Option<&Component>,
        mut update: impl FnMut(i32) -> i32,
    ) {
        let names = self.by_criteria.get(criteria).cloned().unwrap_or_default();
        for name in names {
            let value = update(self.get(holder, &name));
            self.set_with(holder, &name, value, display);
        }
    }
    pub fn has_objectives_for(&self, criteria: &str) -> bool {
        self.by_criteria
            .get(criteria)
            .is_some_and(|objectives| !objectives.is_empty())
    }
    /// Reset all scores for a holder.
    pub fn reset_all(&mut self, holder: &str) {
        if self.scores.remove(holder).is_some() {
            self.holder_order.remove(holder);
        }
    }
    /// Reset one score and forget the holder if no scores remain.
    pub fn reset_one(&mut self, holder: &str, objective: &str) {
        if let Some(scores) = self.scores.get_mut(holder) {
            scores.remove(objective);
            if scores.is_empty() {
                self.reset_all(holder);
            }
        }
    }
    /// Score entries for an objective in `playerScores.forEach` iteration order.
    pub fn list_scores(&self, objective: &str) -> Vec<ScoreEntry> {
        self.holder_order
            .iter()
            .filter_map(|owner| {
                let score = self.scores.get(owner)?.get(objective)?;
                Some(ScoreEntry {
                    owner: owner.to_owned(),
                    value: score.value,
                    display: score.display.clone(),
                    number_format: score.number_format.clone(),
                })
            })
            .collect()
    }
    /// A holder's scores as `(objective, value)` in objective vector order.
    pub fn holder_scores(&self, holder: &str) -> Vec<(&Objective, i32)> {
        let Some(scores) = self.scores.get(holder) else {
            return Vec::new();
        };
        self.objectives
            .iter()
            .filter_map(|objective| Some((objective, scores.get(&objective.name)?.value)))
            .collect()
    }
    /// Tracked holders in vanilla stream order.
    pub fn tracked(&self) -> Vec<String> {
        self.holder_order.stream().map(str::to_owned).collect()
    }
    pub fn team_members(&self) -> impl Iterator<Item = &str> {
        self.team_of.keys().map(String::as_str)
    }
    pub fn team(&self, name: &str) -> Option<&Team> {
        self.teams.iter().find(|team| team.name == name)
    }
    pub fn team_mut(&mut self, name: &str) -> Option<&mut Team> {
        self.teams.iter_mut().find(|team| team.name == name)
    }
    pub fn add_team(&mut self, name: &str) -> &mut Team {
        if self.team(name).is_none() {
            self.teams.push(Team::new(name));
            self.team_order.insert(name);
            self.reorder();
        }
        self.team_mut(name).expect("just added")
    }
    /// Teams in the order used by vanilla stream-based operations and saving.
    pub fn teams_streamed(&self) -> Vec<&Team> {
        self.team_order
            .stream()
            .filter_map(|name| self.team(name))
            .collect()
    }
    /// Remove a team and clear its members' reverse mappings.
    pub fn remove_team(&mut self, name: &str) {
        let Some(at) = self.teams.iter().position(|team| team.name == name) else {
            return;
        };
        let team = self.teams.remove(at);
        for player in team.players.iter() {
            self.team_of.remove(player);
        }
        self.team_order.remove(name);
        self.reorder();
    }
    pub fn players_team(&self, player: &str) -> Option<&Team> {
        self.team(self.team_of.get(player)?)
    }
    /// Leave the old team first, including when the requested team is the same; false if team is absent.
    pub fn add_to_team(&mut self, player: &str, team: &str) -> bool {
        self.remove_from_team(player);
        self.team_of.insert(player.to_owned(), team.to_owned());
        match self.team_mut(team) {
            Some(team) => team.players.insert(player),
            None => false,
        }
    }
    pub fn remove_from_team(&mut self, player: &str) -> bool {
        let Some(team) = self.team_of.remove(player) else {
            return false;
        };
        if let Some(team) = self.team_mut(&team) {
            team.players.remove(player);
        }
        true
    }
    /// Whether two holder names map to the same team.
    pub fn allied(&self, a: &str, b: &str) -> bool {
        match (self.team_of.get(a), self.team_of.get(b)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }
    /// `Scoreboard.entityRemoved` for a dead non-player entity.
    pub fn entity_removed(&mut self, holder: &str) {
        self.reset_all(holder);
        self.remove_from_team(holder);
    }

    /// Encode vanilla `ScoreboardSaveData.Packed` (`data/minecraft/scoreboard.dat`).
    pub fn pack(&self) -> Tag {
        let objectives: Vec<Tag> = self
            .objectives_streamed()
            .into_iter()
            .map(|objective| {
                let mut fields = vec![
                    ("Name", Tag::String(objective.name.as_str().into())),
                    ("DisplayName", component_tag(&objective.display_name)),
                ];
                if objective.criteria != CRITERIA_DUMMY {
                    fields.push((
                        "CriteriaName",
                        Tag::String(objective.criteria.as_str().into()),
                    ));
                }
                if objective.render_type != RenderType::Integer {
                    fields.push(("RenderType", Tag::String(objective.render_type.id().into())));
                }
                if objective.display_auto_update {
                    fields.push(("display_auto_update", Tag::Byte(1)));
                }
                if let Some(format) = &objective.number_format {
                    fields.push(("format", format.to_tag()));
                }
                Tag::compound(fields)
            })
            .collect();
        let mut scores = Vec::new();
        for owner in self.holder_order.stream() {
            let Some(per_objective) = self.scores.get(owner) else {
                continue;
            };
            for objective in self.objectives_streamed() {
                let Some(score) = per_objective.get(&objective.name) else {
                    continue;
                };
                let mut fields = vec![
                    ("Name", Tag::String(owner.into())),
                    ("Objective", Tag::String(objective.name.as_str().into())),
                ];
                if score.value != 0 {
                    fields.push(("Score", Tag::Int(score.value)));
                }
                if score.locked {
                    fields.push(("Locked", Tag::Byte(1)));
                }
                if let Some(display) = &score.display {
                    fields.push(("display", component_tag(display)));
                }
                if let Some(format) = &score.number_format {
                    fields.push(("format", format.to_tag()));
                }
                scores.push(Tag::compound(fields));
            }
        }
        let slots: Vec<(&str, Tag)> = display_slots()
            .iter()
            .filter_map(|slot| {
                let objective = self.display.get(*slot)?;
                self.objective(objective)?;
                Some((*slot, Tag::String(objective.as_str().into())))
            })
            .collect();
        let teams: Vec<Tag> = self
            .teams_streamed()
            .into_iter()
            .map(|team| {
                let mut fields = vec![
                    ("Name", Tag::String(team.name.as_str().into())),
                    ("DisplayName", component_tag(&team.display_name)),
                ];
                if let Some(color) = &team.color {
                    fields.push(("TeamColor", Tag::String(color.as_str().into())));
                }
                if !team.allow_friendly_fire {
                    fields.push(("AllowFriendlyFire", Tag::Byte(0)));
                }
                if !team.see_friendly_invisibles {
                    fields.push(("SeeFriendlyInvisibles", Tag::Byte(0)));
                }
                let empty = Value::String(String::new());
                if canonical_component(&team.prefix) != empty {
                    fields.push(("MemberNamePrefix", component_tag(&team.prefix)));
                }
                if canonical_component(&team.suffix) != empty {
                    fields.push(("MemberNameSuffix", component_tag(&team.suffix)));
                }
                if team.nametag_visibility != Visibility::Always {
                    fields.push((
                        "NameTagVisibility",
                        Tag::String(team.nametag_visibility.name().into()),
                    ));
                }
                if team.death_message_visibility != Visibility::Always {
                    fields.push((
                        "DeathMessageVisibility",
                        Tag::String(team.death_message_visibility.name().into()),
                    ));
                }
                if team.collision_rule != CollisionRule::Always {
                    fields.push((
                        "CollisionRule",
                        Tag::String(team.collision_rule.name().into()),
                    ));
                }
                if !team.players.is_empty() {
                    fields.push((
                        "Players",
                        Tag::List(
                            team.players
                                .iter()
                                .map(|player| Tag::String(player.as_str().into()))
                                .collect(),
                        ),
                    ));
                }
                Tag::compound(fields)
            })
            .collect();
        let mut fields: Vec<(&str, Tag)> = Vec::new();
        if !objectives.is_empty() {
            fields.push(("Objectives", Tag::List(objectives)));
        }
        if !scores.is_empty() {
            fields.push(("PlayerScores", Tag::List(scores)));
        }
        if !slots.is_empty() {
            fields.push(("DisplaySlots", Tag::compound(slots)));
        }
        if !teams.is_empty() {
            fields.push(("Teams", Tag::List(teams)));
        }
        Tag::compound(fields)
    }

    /// Load a vanilla scoreboard save. Invalid entries are skipped as in `ServerScoreboard.load`.
    pub fn unpack(data: &Tag) -> Self {
        let mut board = Scoreboard::default();
        let list = |key: &str| match data.get(key) {
            Some(Tag::List(items)) => items.clone(),
            _ => Vec::new(),
        };
        for objective in list("Objectives") {
            let Some(name) = string_of(objective.get("Name")) else {
                continue;
            };
            let criteria = string_of(objective.get("CriteriaName"))
                .unwrap_or_else(|| CRITERIA_DUMMY.to_owned());
            board.add_objective(
                &name,
                &criteria,
                objective
                    .get("DisplayName")
                    .map_or_else(|| literal(&name), tag_to_json),
                RenderType::by_id(&string_of(objective.get("RenderType")).unwrap_or_default()),
                bool_of(objective.get("display_auto_update"), false),
                objective.get("format").and_then(NumberFormat::from_tag),
            );
        }
        for score in list("PlayerScores") {
            let (Some(owner), Some(objective)) = (
                string_of(score.get("Name")),
                string_of(score.get("Objective")),
            ) else {
                continue;
            };
            if board.objective(&objective).is_none() {
                continue;
            }
            *board.score_mut(&owner, &objective) = Score {
                value: int_of(score.get("Score")).unwrap_or(0),
                locked: bool_of(score.get("Locked"), false),
                display: score.get("display").map(tag_to_json),
                number_format: score.get("format").and_then(NumberFormat::from_tag),
            };
        }
        if let Some(Tag::Compound(slots)) = data.get("DisplaySlots") {
            for (key, value) in slots {
                let slot = key.to_string_lossy();
                if let Tag::String(objective) = value
                    && display_slot_valid(&slot)
                {
                    let objective = objective.to_string_lossy();
                    if board.objective(&objective).is_some() {
                        board.display.insert(slot, objective);
                    }
                }
            }
        }
        for saved_team in list("Teams") {
            let Some(name) = string_of(saved_team.get("Name")) else {
                continue;
            };
            let team = board.add_team(&name);
            if let Some(display_name) = saved_team.get("DisplayName") {
                team.display_name = tag_to_json(display_name);
            }
            team.color = string_of(saved_team.get("TeamColor"))
                .filter(|color| team_color_names::is_team_color_name(color));
            team.allow_friendly_fire = bool_of(saved_team.get("AllowFriendlyFire"), true);
            team.see_friendly_invisibles = bool_of(saved_team.get("SeeFriendlyInvisibles"), true);
            team.prefix = saved_team
                .get("MemberNamePrefix")
                .map_or_else(|| literal(""), tag_to_json);
            team.suffix = saved_team
                .get("MemberNameSuffix")
                .map_or_else(|| literal(""), tag_to_json);
            team.nametag_visibility = string_of(saved_team.get("NameTagVisibility"))
                .and_then(|value| Visibility::by_name(&value))
                .unwrap_or(Visibility::Always);
            team.death_message_visibility = string_of(saved_team.get("DeathMessageVisibility"))
                .and_then(|value| Visibility::by_name(&value))
                .unwrap_or(Visibility::Always);
            team.collision_rule = string_of(saved_team.get("CollisionRule"))
                .and_then(|value| CollisionRule::by_name(&value))
                .unwrap_or(CollisionRule::Always);
            if let Some(Tag::List(players)) = saved_team.get("Players") {
                for player in players {
                    if let Tag::String(player) = player {
                        board.add_to_team(&player.to_string_lossy(), &name);
                    }
                }
            }
        }
        board
    }
}

fn string_of(tag: Option<&Tag>) -> Option<String> {
    match tag {
        Some(Tag::String(value)) => Some(value.to_string_lossy()),
        _ => None,
    }
}
fn bool_of(tag: Option<&Tag>, default: bool) -> bool {
    match tag {
        Some(Tag::Byte(value)) => *value != 0,
        _ => default,
    }
}
fn int_of(tag: Option<&Tag>) -> Option<i32> {
    match tag {
        Some(Tag::Int(value)) => Some(*value),
        Some(Tag::Short(value)) => Some(i32::from(*value)),
        Some(Tag::Byte(value)) => Some(i32::from(*value)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Scoreboard {
        let mut board = Scoreboard::default();
        board.add_objective(
            "kills",
            "playerKillCount",
            literal("Kills"),
            RenderType::Integer,
            false,
            None,
        );
        board.add_objective(
            "hp",
            "health",
            literal("HP"),
            RenderType::Hearts,
            true,
            Some(NumberFormat::Blank),
        );
        board
    }

    #[test]
    fn scores_objectives_and_holder_lifecycle_match_vanilla() {
        let mut board = board();
        assert!(!board.add_objective(
            "kills",
            CRITERIA_DUMMY,
            literal("x"),
            RenderType::Integer,
            false,
            None
        ));
        assert!(board.score_mut("Steve", "kills").locked);
        board.for_all_objectives("playerKillCount", "Steve", |value| value + 1);
        board.for_all_objectives("playerKillCount", "Steve", |value| value + 1);
        assert_eq!(board.get("Steve", "kills"), 2);
        assert_eq!(board.add("Steve", "kills", 5), 7);
        assert_eq!(board.holder_scores("Steve").len(), 1);
        board.set("Alex", "hp", 20);
        let mut tracked = board.tracked();
        tracked.sort();
        assert_eq!(tracked, vec!["Alex".to_owned(), "Steve".to_owned()]);
        board.reset_one("Alex", "hp");
        assert_eq!(board.tracked(), vec!["Steve".to_owned()]);
        board.set_display_objective("sidebar", Some("kills"));
        board.remove_objective("kills");
        assert!(board.display_objective("sidebar").is_none());
        assert_eq!(board.tracked(), vec!["Steve".to_owned()]);
        assert!(board.holder_scores("Steve").is_empty());
    }

    #[test]
    fn teams_membership_and_server_rules_match_vanilla() {
        let mut board = Scoreboard::default();
        board.add_team("red");
        board.add_team("blue");
        assert!(board.add_to_team("Steve", "red"));
        assert!(board.add_to_team("Steve", "red"));
        assert!(board.add_to_team("Alex", "red"));
        assert!(board.allied("Steve", "Alex"));
        assert!(teams_allied(
            board.players_team("Steve"),
            board.players_team("Alex")
        ));
        assert!(teams_allow_damage(
            board.players_team("Steve"),
            board.players_team("Alex")
        ));
        board.team_mut("red").unwrap().allow_friendly_fire = false;
        assert!(!teams_allow_damage(
            board.players_team("Steve"),
            board.players_team("Alex")
        ));
        assert!(board.add_to_team("Steve", "blue"));
        assert!(!board.allied("Steve", "Alex"));
        assert_eq!(board.team("red").map(|team| team.players.len()), Some(1));
        assert!(board.remove_from_team("Alex"));
        assert!(!board.remove_from_team("Alex"));
        board.remove_team("blue");
        assert!(board.players_team("Steve").is_none());
    }

    #[test]
    fn pack_round_trip_and_nbt_binary_preserve_vanilla_state() {
        let mut board = board();
        board.set("Steve", "kills", 3);
        board.score_mut("Steve", "kills").locked = false;
        board.score_mut("Steve", "kills").display =
            Some(serde_json::json!({"text": "S", "color": "red"}));
        board.score_mut("Steve", "kills").number_format =
            Some(NumberFormat::Fixed(literal("many")));
        board.set_display_objective("sidebar.team.red", Some("kills"));
        let team = board.add_team("red");
        team.color = Some("red".into());
        team.prefix = literal("[R] ");
        team.allow_friendly_fire = false;
        team.nametag_visibility = Visibility::HideForOtherTeams;
        team.collision_rule = CollisionRule::PushOwnTeam;
        board.add_to_team("Steve", "red");
        board.objective_mut("hp").unwrap().number_format = Some(NumberFormat::Styled(
            serde_json::json!({"color": "gold", "bold": true}),
        ));
        let packed = board.pack();
        assert_eq!(Scoreboard::unpack(&packed), board);
        let bytes = crate::nbt::write_named(
            &crate::nbt::NamedTag {
                name: "".into(),
                tag: packed,
            },
            Default::default(),
        )
        .unwrap();
        let (named, _) = crate::nbt::read_named(&bytes, Default::default()).unwrap();
        assert_eq!(Scoreboard::unpack(&named.tag), board);
    }

    #[test]
    fn number_format_codec_keeps_fixed_and_styled_component_data() {
        for format in [
            NumberFormat::Blank,
            NumberFormat::Fixed(literal("many")),
            NumberFormat::Styled(serde_json::json!({"color": "gold", "bold": true})),
        ] {
            assert_eq!(NumberFormat::from_tag(&format.to_tag()), Some(format));
        }
        assert_eq!(NumberFormat::sidebar_default().to_json()["color"], "red");
        assert_eq!(
            NumberFormat::player_list_default().to_json()["color"],
            "yellow"
        );
    }

    #[test]
    fn save_omits_objective_defaults_but_keeps_locked_zero_score() {
        let mut board = Scoreboard::default();
        assert_eq!(
            board.pack(),
            Tag::compound(std::iter::empty::<(&str, Tag)>())
        );
        board.add_objective(
            "plain",
            CRITERIA_DUMMY,
            literal("Plain"),
            RenderType::Integer,
            false,
            None,
        );
        board.score_mut("Alex", "plain");
        let packed = board.pack();
        let Tag::List(objectives) = packed.get("Objectives").unwrap() else {
            panic!("Objectives must be an NBT list");
        };
        assert!(objectives[0].get("CriteriaName").is_none());
        assert!(objectives[0].get("RenderType").is_none());
        assert!(objectives[0].get("display_auto_update").is_none());
        let Tag::List(scores) = packed.get("PlayerScores").unwrap() else {
            panic!("PlayerScores must be an NBT list");
        };
        assert!(scores[0].get("Score").is_none());
        assert_eq!(scores[0].get("Locked"), Some(&Tag::Byte(1)));
        assert_eq!(Scoreboard::unpack(&packed), board);
    }

    #[test]
    fn unpack_skips_invalid_entries_and_uses_vanilla_fallbacks() {
        let data = Tag::compound([
            (
                "Objectives",
                Tag::List(vec![
                    Tag::String("not a compound".into()),
                    Tag::compound([("Name", Tag::String("ok".into()))]),
                ]),
            ),
            (
                "PlayerScores",
                Tag::List(vec![
                    Tag::compound([
                        ("Name", Tag::String("Alex".into())),
                        ("Objective", Tag::String("missing".into())),
                    ]),
                    Tag::compound([
                        ("Name", Tag::String("Alex".into())),
                        ("Objective", Tag::String("ok".into())),
                        ("Score", Tag::String("wrong type".into())),
                    ]),
                ]),
            ),
            (
                "DisplaySlots",
                Tag::compound([
                    ("sidebar.team.unknown", Tag::String("ok".into())),
                    ("sidebar", Tag::String("missing".into())),
                ]),
            ),
            (
                "Teams",
                Tag::List(vec![
                    Tag::String("not a team".into()),
                    Tag::compound([
                        ("Name", Tag::String("red".into())),
                        ("TeamColor", Tag::String("magenta".into())),
                        ("NameTagVisibility", Tag::String("unknown".into())),
                        (
                            "Players",
                            Tag::List(vec![Tag::String("Alex".into()), Tag::Int(7)]),
                        ),
                    ]),
                ]),
            ),
        ]);
        let board = Scoreboard::unpack(&data);
        assert_eq!(board.objectives.len(), 1);
        assert_eq!(board.objective("ok").unwrap().criteria, CRITERIA_DUMMY);
        assert_eq!(board.objective("ok").unwrap().display_name, literal("ok"));
        assert_eq!(board.score("Alex", "ok").unwrap().value, 0);
        assert!(!board.score("Alex", "ok").unwrap().locked);
        assert!(board.display.is_empty());
        let team = board.team("red").unwrap();
        assert_eq!(team.color, None);
        assert_eq!(team.nametag_visibility, Visibility::Always);
        assert_eq!(
            team.players.iter().map(String::as_str).collect::<Vec<_>>(),
            ["Alex"]
        );
    }
}
