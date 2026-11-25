// src/ingest/cs2/teams.rs
//! Normalizacja nazw drużyn CS2 pomiędzy zbiorem Kaggle CSV a PandaScore API.

/// Mapowanie popularnych skrótów i wariantów nazw drużyn CS2 na kanoniczną nazwę.
const CS2_TEAM_ALIASES: &[(&str, &str)] = &[
    ("navi", "Natus Vincere"),
    ("na'vi", "Natus Vincere"),
    ("natus vincere", "Natus Vincere"),
    ("vp", "Virtus.pro"),
    ("virtus pro", "Virtus.pro"),
    ("virtus.pro", "Virtus.pro"),
    ("faze", "FaZe Clan"),
    ("faze clan", "FaZe Clan"),
    ("vitality", "Team Vitality"),
    ("team vitality", "Team Vitality"),
    ("nip", "Ninjas in Pyjamas"),
    ("ninjas in pyjamas", "Ninjas in Pyjamas"),
    ("g2", "G2 Esports"),
    ("g2 esports", "G2 Esports"),
    ("mouz", "MOUZ"),
    ("mousesports", "MOUZ"),
    ("spirit", "Team Spirit"),
    ("team spirit", "Team Spirit"),
    ("liquid", "Team Liquid"),
    ("team liquid", "Team Liquid"),
    ("falcons", "Team Falcons"),
    ("team falcons", "Team Falcons"),
    ("furia", "FURIA"),
    ("furia esports", "FURIA"),
    ("mongolz", "The MongolZ"),
    ("the mongolz", "The MongolZ"),
    ("col", "Complexity"),
    ("complexity", "Complexity"),
    ("complexity gaming", "Complexity"),
    ("c9", "Cloud9"),
    ("cloud9", "Cloud9"),
    ("fnatic", "Fnatic"),
    ("big", "BIG"),
    ("ence", "ENCE"),
    ("heroic", "HEROIC"),
    ("astralis", "Astralis"),
    ("ef", "Eternal Fire"),
    ("eternal fire", "Eternal Fire"),
    ("3dmax", "3DMAX"),
    ("aurora", "Aurora"),
    ("aurora gaming", "Aurora"),
    ("gl", "GamerLegion"),
    ("gamerlegion", "GamerLegion"),
    ("pain", "paiN Gaming"),
    ("pain gaming", "paiN Gaming"),
    ("mibr", "MIBR"),
    ("imperial", "Imperial"),
    ("imperial esports", "Imperial"),
    ("betboom", "BetBoom Team"),
    ("betboom team", "BetBoom Team"),
    ("saw", "SAW"),
];

/// Zwraca ujednoliconą nazwę drużyny CS2, aby rekordy z Kaggle CSV i PandaScore scalały się bez duplikatów.
pub fn normalize_cs2_team_name(raw_name: &str) -> String {
    let trimmed = raw_name.trim();
    let lowered = trimmed.to_ascii_lowercase();
    for (alias, canonical) in CS2_TEAM_ALIASES {
        if lowered == *alias {
            return (*canonical).to_string();
        }
    }
    trimmed.to_string()
}
