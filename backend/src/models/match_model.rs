// src/models/match_model.rs
//! Modele meczów, kursów oraz wzbogacony widok meczu zwracany przez REST API.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{types::Json, FromRow};
use utoipa::ToSchema;

/// Statystyki drużyny z jednego meczu NBA (połączenie `Base` oraz `Advanced` z `stats.nba.com/stats/teamgamelogs`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct BasketballTeamStats {
    // --- Base (box-score) ---
    /// Łączny czas gry (minuty, `> 50.0` wskazuje dogrywkę).
    pub min: Option<f64>,
    /// Celne rzuty z gry (`FGM`).
    pub fgm: Option<f64>,
    /// Oddane rzuty z gry (`FGA`).
    pub fga: Option<f64>,
    /// Skuteczność rzutów z gry (`FG%`).
    pub fg_pct: Option<f64>,
    /// Celne rzuty za 3 punkty (`3PM`).
    pub fg3m: Option<f64>,
    /// Oddane rzuty za 3 punkty (`3PA`).
    pub fg3a: Option<f64>,
    /// Skuteczność rzutów za 3 punkty (`3P%`).
    pub fg3_pct: Option<f64>,
    /// Celne rzuty wolne (`FTM`).
    pub ftm: Option<f64>,
    /// Oddane rzuty wolne (`FTA`).
    pub fta: Option<f64>,
    /// Skuteczność rzutów wolnych (`FT%`).
    pub ft_pct: Option<f64>,
    /// Zbiórki w ataku (`OREB`).
    pub oreb: Option<f64>,
    /// Zbiórki w obronie (`DREB`).
    pub dreb: Option<f64>,
    /// Zbiórki łącznie (`REB`).
    pub reb: Option<f64>,
    /// Asysty (`AST`).
    pub ast: Option<f64>,
    /// Straty (`TOV`).
    pub tov: Option<f64>,
    /// Przechwyty (`STL`).
    pub stl: Option<f64>,
    /// Bloki (`BLK`).
    pub blk: Option<f64>,
    /// Próby zablokowane przez rywala (`BLKA`).
    pub blka: Option<f64>,
    /// Faule popełnione (`PF`).
    pub pf: Option<f64>,
    /// Faule wymuszone (`PFD`).
    pub pfd: Option<f64>,
    /// Wskaźnik plus/minus (`+/-`).
    pub plus_minus: Option<f64>,
    // --- Advanced (metryki efektywności) ---
    /// Offensive Rating (punkty na 100 posiadań).
    pub off_rating: Option<f64>,
    /// Defensive Rating (stracone punkty na 100 posiadań).
    pub def_rating: Option<f64>,
    /// Net Rating (`off_rating - def_rating`).
    pub net_rating: Option<f64>,
    /// Tempo gry (posiadania na 48 minut).
    pub pace: Option<f64>,
    /// Effective Field Goal % (`eFG%`).
    pub efg_pct: Option<f64>,
    /// True Shooting % (`TS%`).
    pub ts_pct: Option<f64>,
    /// Team Turnover % (`TM_TOV%`).
    pub tm_tov_pct: Option<f64>,
    /// Offensive Rebound % (`OREB%`).
    pub oreb_pct: Option<f64>,
    /// Player Impact Estimate (`PIE`).
    pub pie: Option<f64>,
}

/// Statystyki drużyny z jednego meczu piłkarskiego (`football-data.co.uk`, post-match).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FootballTeamStats {
    /// Expected Goals (`HxG` / `AxG`), jeśli dostępne w pliku CSV.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 1.65)]
    pub xg: Option<f64>,
    /// Łączna liczba strzałów (`HS` / `AS`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 14)]
    pub shots: Option<i32>,
    /// Strzały celne (`HST` / `AST`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 6)]
    pub shots_on_target: Option<i32>,
    /// Rzuty rożne (`HC` / `AC`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 7)]
    pub corners: Option<i32>,
    /// Faule (`HF` / `AF`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 11)]
    pub fouls: Option<i32>,
    /// Żółte kartki (`HY` / `AY`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 2)]
    pub yellow_cards: Option<i32>,
    /// Czerwone kartki (`HR` / `AR`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 0)]
    pub red_cards: Option<i32>,
}

/// Przedmeczowy ranking siły drużyn na dzień przed meczem (`clubelo.com`, `D - 1`, pre-match).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FootballPreMatchElo {
    /// Rating Elo gospodarza z dnia `D - 1` (lub aktualny dla meczu `SCHEDULED`).
    #[schema(example = 1985.4)]
    pub home_elo: Option<f64>,
    /// Rating Elo gościa z dnia `D - 1` (lub aktualny dla meczu `SCHEDULED`).
    #[schema(example = 1870.2)]
    pub away_elo: Option<f64>,
    /// Różnica ratingów (`home_elo - away_elo`).
    #[schema(example = 115.2)]
    pub elo_diff: Option<f64>,
    /// Miejsce gospodarza w rankingu ClubElo.
    #[schema(example = 2)]
    pub home_rank: Option<i32>,
    /// Miejsce gościa w rankingu ClubElo.
    #[schema(example = 9)]
    pub away_rank: Option<i32>,
    /// Opcjonalne prawdopodobieństwo wygranej gospodarza z `/Fixtures` (tylko dla `SCHEDULED`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.56)]
    pub clubelo_prob_home: Option<f64>,
    /// Opcjonalne prawdopodobieństwo remisu z `/Fixtures` (tylko dla `SCHEDULED`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.24)]
    pub clubelo_prob_draw: Option<f64>,
    /// Opcjonalne prawdopodobieństwo wygranej gościa z `/Fixtures` (tylko dla `SCHEDULED`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.20)]
    pub clubelo_prob_away: Option<f64>,
}

/// Wynik pojedynczej mapy w serii CS2 (nazwa mapy, punkty rundowe i zwycięzca).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Cs2MapResult {
    /// Nazwa mapy, np. `"Mirage"`, `"Inferno"`, `"Nuke"`.
    #[schema(example = "Mirage")]
    pub map_name: String,
    /// Liczba rund wygranych przez stronę `HOME` (z Kaggle CSV; brak w PandaScore Free).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 13)]
    pub home_score: Option<i32>,
    /// Liczba rund wygranych przez stronę `AWAY` (z Kaggle CSV; brak w PandaScore Free).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 9)]
    pub away_score: Option<i32>,
    /// `"HOME"`, `"AWAY"` lub `"DRAW"` (jeśli mapa zakończona remisem).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "HOME")]
    pub winner: Option<String>,
}

/// Zawodnik drużyny CS2 zapisany w składzie meczowym lub turniejowym.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Cs2Player {
    /// Zewnętrzne ID zawodnika (PandaScore lub Kaggle/HLTV).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 11893)]
    pub external_id: Option<i64>,
    /// Nick zawodnika.
    #[schema(example = "ZywOo")]
    pub name: String,
}

/// Statystyki indywidualne zawodnika z serii CS2 (źródło pomeczowe, np. Kaggle CSV).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Cs2PlayerStats {
    /// `"HOME"` lub `"AWAY"` (zgodne z kierunkiem stron w rekordzie meczu).
    #[schema(example = "HOME")]
    pub team_side: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 11893)]
    pub player_id: Option<i64>,
    #[schema(example = "ZywOo")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 42)]
    pub kills: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 29)]
    pub deaths: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 9)]
    pub assists: Option<i32>,
    /// Average Damage per Round (`ADR`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 91.4)]
    pub adr: Option<f64>,
    /// Procent rund z udziałem w Kill/Assist/Survive/Trade (`KAST`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 78.2)]
    pub kast: Option<f64>,
    /// Różnica `kills - deaths`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 13)]
    pub kddiff: Option<i32>,
}

/// Pojedynczy krok fazy veto (ban/pick/decider) w meczu CS2 – opcjonalne pod przyszłe wzbogacenie.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Cs2VetoAction {
    /// Kolejność kroku w fazie veto (od 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 1)]
    pub order: Option<u8>,
    /// Strona wykonująca akcję (`"HOME"`, `"AWAY"` lub `"DECIDER"`).
    #[schema(example = "HOME")]
    pub team_side: String,
    /// Typ akcji, np. `"ban"`, `"pick"` lub `"decider"`.
    #[schema(example = "ban")]
    pub action: String,
    /// Nazwa mapy, której dotyczy krok veto.
    #[schema(example = "Ancient")]
    pub map_name: String,
}

/// Szczegóły specyficzne dla dyscypliny, przechowywane w kolumnie JSONB `match_details`.
/// Wariant rozpoznawany jest po polu `sport_type` (np. `"football"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "sport_type", rename_all = "snake_case")]
pub enum MatchDetails {
    /// Piłka nożna: wynik do przerwy, kolejka oraz opcjonalne statystyki meczowe i Elo przedmeczowe.
    Football {
        ht_home: Option<i32>,
        ht_away: Option<i32>,
        round: String,
        /// Box ogranicza rozmiar wariantu enuma `MatchDetails` dla pozostałych dyscyplin.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        home_stats: Option<Box<FootballTeamStats>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        away_stats: Option<Box<FootballTeamStats>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pre_match_elo: Option<Box<FootballPreMatchElo>>,
    },
    /// Koszykówka: punkty w kwartach (gospodarz, gość), dogrywka oraz opcjonalne statystyki NBA.
    Basketball {
        #[schema(value_type = Vec<[i32; 2]>)]
        quarters: Vec<(i32, i32)>,
        overtime: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        season_type: Option<String>,
        /// Box ogranicza rozmiar wariantu enuma `MatchDetails` dla pozostałych dyscyplin.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        home_stats: Option<Box<BasketballTeamStats>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        away_stats: Option<Box<BasketballTeamStats>>,
    },
    /// Tenis: nawierzchnia, format meczu, gemy w setach oraz liczba asów.
    Tennis {
        surface: String,
        best_of: u8,
        #[schema(value_type = Vec<[u8; 2]>)]
        sets: Vec<(u8, u8)>,
        home_aces: Option<u16>,
        away_aces: Option<u16>,
    },
    /// Counter-Strike 2: format serii, turniej, mapy, składy, statystyki graczy oraz opcjonalne veto.
    Cs2 {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        best_of: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tournament_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tournament_tier: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        maps: Vec<Cs2MapResult>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        home_roster: Vec<Cs2Player>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        away_roster: Vec<Cs2Player>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        player_stats: Vec<Cs2PlayerStats>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        veto: Vec<Cs2VetoAction>,
    },
}

/// Mecz (przeszły, trwający lub nadchodzący) – wiersz tabeli `matches`.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Match {
    pub id: i32,
    pub competition_id: i32,
    pub season: String,
    pub match_date: DateTime<Utc>,
    /// "SCHEDULED", "LIVE", "FINISHED", "POSTPONED" lub "CANCELLED".
    pub status: String,
    pub home_participant_id: i32,
    pub away_participant_id: i32,
    /// Wynik główny: bramki, punkty lub wygrane sety (zależnie od sportu).
    pub home_score: Option<i32>,
    pub away_score: Option<i32>,
    /// "HOME", "AWAY" lub "DRAW" – ustawiane po zakończeniu meczu.
    pub winner: Option<String>,
    pub match_details: Json<MatchDetails>,
    /// Identyfikatory z zewnętrznych API (ułatwiają aktualizację wyników).
    pub external_ids: Json<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Kursy bukmacherskie dla meczu (`1X2` dla piłki nożnej, `12` dla koszykówki, tenisa i CS2).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MatchOdds {
    pub id: i32,
    pub match_id: i32,
    /// Źródło kursu; giełda używa kursu `"MARKET_AVG"`.
    pub bookmaker: String,
    pub odds_home: Decimal,
    /// `None` dla sportów bez remisu (`BASKETBALL`, `TENNIS`, `CS2`).
    pub odds_draw: Option<Decimal>,
    pub odds_away: Decimal,
    pub updated_at: DateTime<Utc>,
}

/// Widok meczu zwracany przez `GET /api/v1/matches` oraz `GET /api/v1/matches/{id}`,
/// wzbogacony o nazwy uczestników, polimorficzne `match_details` i kursy `MARKET_AVG`.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct MatchWithDetailsDto {
    #[schema(example = 4)]
    pub id: i32,
    /// Dyscyplina: `"FOOTBALL"`, `"BASKETBALL"`, `"TENNIS"` lub `"CS2"`.
    #[schema(example = "FOOTBALL")]
    pub sport: String,
    #[schema(example = "Premier League")]
    pub competition_name: String,
    #[schema(example = "2026-2027")]
    pub season: String,
    pub match_date: DateTime<Utc>,
    /// Status meczu: `"SCHEDULED"`, `"LIVE"`, `"FINISHED"`, `"POSTPONED"` lub `"CANCELLED"`.
    #[schema(example = "SCHEDULED")]
    pub status: String,
    #[schema(example = 2)]
    pub home_participant_id: i32,
    #[schema(example = "Chelsea FC")]
    pub home_name: String,
    #[schema(example = 1)]
    pub away_participant_id: i32,
    #[schema(example = "Arsenal FC")]
    pub away_name: String,
    /// Wynik gospodarza (bramki / punkty / wygrane sety / wygrane mapy); `null` przed zakończeniem.
    pub home_score: Option<i32>,
    /// Wynik gościa (bramki / punkty / wygrane sety / wygrane mapy); `null` przed zakończeniem.
    pub away_score: Option<i32>,
    /// Zwycięzca zakończonego meczu: `"HOME"`, `"AWAY"` lub `"DRAW"`.
    pub winner: Option<String>,
    /// Szczegóły specyficzne dla dyscypliny (rozróżniane polem `sport_type`).
    #[schema(value_type = MatchDetails)]
    pub match_details: Json<MatchDetails>,
    /// Kurs `MARKET_AVG` na wygraną gospodarza (`null`, jeśli brak kursów).
    #[schema(example = 2.65)]
    pub odds_home: Option<Decimal>,
    /// Kurs `MARKET_AVG` na remis (`null` dla sportów bez remisu lub przed wyceną).
    #[schema(example = 3.40)]
    pub odds_draw: Option<Decimal>,
    /// Kurs `MARKET_AVG` na wygraną gościa (`null`, jeśli brak kursów).
    #[schema(example = 2.60)]
    pub odds_away: Option<Decimal>,
}
