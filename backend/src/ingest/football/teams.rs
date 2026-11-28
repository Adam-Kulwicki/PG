// src/ingest/football/teams.rs
//! Kanoniczna normalizacja nazw drużyn, slugi ClubElo oraz konfiguracja lig Top 5.

/// Specyfikacja ligi piłkarskiej z identyfikatorami w obsługiwanych źródłach danych.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeagueSpec {
    /// Wewnętrzny kod ligi zgodny z `Football-Data.org` i kolumną `competitions.external_code`.
    pub code: &'static str,
    pub name: &'static str,
    pub country: &'static str,
    /// Kod dywizji w plikach CSV `football-data.co.uk` (`E0`, `SP1`, `D1`, `I1`, `F1`).
    pub football_data_uk: &'static str,
    /// Kod kraju używany przez `clubelo.com` (np. `ENG`).
    pub clubelo_country: &'static str,
}

/// Obsługiwane ligi Top 5 Europy (`PL`, `PD`, `BL1`, `SA`, `FL1`).
pub const TOP5_LEAGUES: [LeagueSpec; 5] = [
    LeagueSpec {
        code: "PL",
        name: "Premier League",
        country: "England",
        football_data_uk: "E0",
        clubelo_country: "ENG",
    },
    LeagueSpec {
        code: "PD",
        name: "Primera Division",
        country: "Spain",
        football_data_uk: "SP1",
        clubelo_country: "ESP",
    },
    LeagueSpec {
        code: "BL1",
        name: "Bundesliga",
        country: "Germany",
        football_data_uk: "D1",
        clubelo_country: "GER",
    },
    LeagueSpec {
        code: "SA",
        name: "Serie A",
        country: "Italy",
        football_data_uk: "I1",
        clubelo_country: "ITA",
    },
    LeagueSpec {
        code: "FL1",
        name: "Ligue 1",
        country: "France",
        football_data_uk: "F1",
        clubelo_country: "FRA",
    },
];

/// Zwraca specyfikację ligi na podstawie kodu wewnętrznego (`PL`, `PD`, `BL1`, `SA`, `FL1`)
/// lub kodu dywizji `football-data.co.uk` (`E0`, `SP1`, `D1`, `I1`, `F1`).
pub fn league_spec(code: &str) -> Option<&'static LeagueSpec> {
    let trimmed = code.trim();
    TOP5_LEAGUES.iter().find(|spec| {
        spec.code.eq_ignore_ascii_case(trimmed)
            || spec.football_data_uk.eq_ignore_ascii_case(trimmed)
    })
}

struct TeamEntry {
    /// Kanoniczna nazwa zapisywana w tabeli `participants` (zgodna z `Football-Data.org`).
    canonical: &'static str,
    /// Slug używany w endpointach `http://api.clubelo.com/{slug}`.
    clubelo: &'static str,
    /// Warianty nazw z Understat, ClubElo CSV oraz skrótów ligowych.
    aliases: &'static [&'static str],
}

/// Wspólny słownik drużyn z Top 5 lig europejskich (sezony 2014/15–2025/26).
const TEAMS: &[TeamEntry] = &[
    // --- Premier League (PL / ENG) ---
    TeamEntry {
        canonical: "Arsenal FC",
        clubelo: "Arsenal",
        aliases: &["Arsenal"],
    },
    TeamEntry {
        canonical: "Aston Villa FC",
        clubelo: "AstonVilla",
        aliases: &["Aston Villa"],
    },
    TeamEntry {
        canonical: "AFC Bournemouth",
        clubelo: "Bournemouth",
        aliases: &["Bournemouth"],
    },
    TeamEntry {
        canonical: "Brentford FC",
        clubelo: "Brentford",
        aliases: &["Brentford"],
    },
    TeamEntry {
        canonical: "Brighton & Hove Albion FC",
        clubelo: "Brighton",
        aliases: &["Brighton", "Brighton & Hove Albion"],
    },
    TeamEntry {
        canonical: "Burnley FC",
        clubelo: "Burnley",
        aliases: &["Burnley"],
    },
    TeamEntry {
        canonical: "Cardiff City FC",
        clubelo: "Cardiff",
        aliases: &["Cardiff", "Cardiff City"],
    },
    TeamEntry {
        canonical: "Chelsea FC",
        clubelo: "Chelsea",
        aliases: &["Chelsea"],
    },
    TeamEntry {
        canonical: "Coventry City FC",
        clubelo: "Coventry",
        aliases: &["Coventry", "Coventry City"],
    },
    TeamEntry {
        canonical: "Crystal Palace FC",
        clubelo: "CrystalPalace",
        aliases: &["Crystal Palace"],
    },
    TeamEntry {
        canonical: "Everton FC",
        clubelo: "Everton",
        aliases: &["Everton"],
    },
    TeamEntry {
        canonical: "Fulham FC",
        clubelo: "Fulham",
        aliases: &["Fulham"],
    },
    TeamEntry {
        canonical: "Huddersfield Town AFC",
        clubelo: "Huddersfield",
        aliases: &["Huddersfield", "Huddersfield Town"],
    },
    TeamEntry {
        canonical: "Hull City AFC",
        clubelo: "Hull",
        aliases: &["Hull", "Hull City"],
    },
    TeamEntry {
        canonical: "Ipswich Town FC",
        clubelo: "Ipswich",
        aliases: &["Ipswich", "Ipswich Town"],
    },
    TeamEntry {
        canonical: "Leeds United FC",
        clubelo: "Leeds",
        aliases: &["Leeds", "Leeds United"],
    },
    TeamEntry {
        canonical: "Leicester City FC",
        clubelo: "Leicester",
        aliases: &["Leicester", "Leicester City"],
    },
    TeamEntry {
        canonical: "Liverpool FC",
        clubelo: "Liverpool",
        aliases: &["Liverpool"],
    },
    TeamEntry {
        canonical: "Luton Town FC",
        clubelo: "Luton",
        aliases: &["Luton", "Luton Town"],
    },
    TeamEntry {
        canonical: "Manchester City FC",
        clubelo: "ManCity",
        aliases: &["Manchester City", "Man City"],
    },
    TeamEntry {
        canonical: "Manchester United FC",
        clubelo: "ManUnited",
        aliases: &["Manchester United", "Man United", "Man Utd"],
    },
    TeamEntry {
        canonical: "Middlesbrough FC",
        clubelo: "Middlesbrough",
        aliases: &["Middlesbrough"],
    },
    TeamEntry {
        canonical: "Newcastle United FC",
        clubelo: "Newcastle",
        aliases: &["Newcastle", "Newcastle United"],
    },
    TeamEntry {
        canonical: "Norwich City FC",
        clubelo: "Norwich",
        aliases: &["Norwich", "Norwich City"],
    },
    TeamEntry {
        canonical: "Nottingham Forest FC",
        clubelo: "Forest",
        aliases: &["Nottingham Forest", "Forest", "Nott'm Forest"],
    },
    TeamEntry {
        canonical: "Queens Park Rangers FC",
        clubelo: "QPR",
        aliases: &["Queens Park Rangers", "QPR"],
    },
    TeamEntry {
        canonical: "Sheffield United FC",
        clubelo: "SheffieldUnited",
        aliases: &["Sheffield United", "Sheffield Utd"],
    },
    TeamEntry {
        canonical: "Southampton FC",
        clubelo: "Southampton",
        aliases: &["Southampton"],
    },
    TeamEntry {
        canonical: "Stoke City FC",
        clubelo: "Stoke",
        aliases: &["Stoke", "Stoke City"],
    },
    TeamEntry {
        canonical: "Sunderland AFC",
        clubelo: "Sunderland",
        aliases: &["Sunderland"],
    },
    TeamEntry {
        canonical: "Swansea City AFC",
        clubelo: "Swansea",
        aliases: &["Swansea", "Swansea City"],
    },
    TeamEntry {
        canonical: "Tottenham Hotspur FC",
        clubelo: "Tottenham",
        aliases: &["Tottenham", "Tottenham Hotspur", "Spurs"],
    },
    TeamEntry {
        canonical: "Watford FC",
        clubelo: "Watford",
        aliases: &["Watford"],
    },
    TeamEntry {
        canonical: "West Bromwich Albion FC",
        clubelo: "WestBrom",
        aliases: &["West Bromwich Albion", "West Brom"],
    },
    TeamEntry {
        canonical: "West Ham United FC",
        clubelo: "WestHam",
        aliases: &["West Ham", "West Ham United"],
    },
    TeamEntry {
        canonical: "Wolverhampton Wanderers FC",
        clubelo: "Wolves",
        aliases: &["Wolverhampton Wanderers", "Wolves"],
    },
    // --- La Liga (PD / ESP) ---
    TeamEntry {
        canonical: "Athletic Club",
        clubelo: "athletic-club",
        aliases: &["Athletic Bilbao", "Ath Bilbao", "Bilbao"],
    },
    TeamEntry {
        canonical: "Club Atlético de Madrid",
        clubelo: "atletico",
        aliases: &[
            "Atletico Madrid",
            "Atlético Madrid",
            "Ath Madrid",
            "Atletico",
            "Atlético",
        ],
    },
    TeamEntry {
        canonical: "CA Osasuna",
        clubelo: "Osasuna",
        aliases: &["Osasuna"],
    },
    TeamEntry {
        canonical: "RC Celta de Vigo",
        clubelo: "rc-celta",
        aliases: &["Celta Vigo", "Celta"],
    },
    TeamEntry {
        canonical: "Deportivo Alavés",
        clubelo: "Alaves",
        aliases: &["Alaves", "Alavés"],
    },
    TeamEntry {
        canonical: "Elche CF",
        clubelo: "Elche",
        aliases: &["Elche"],
    },
    TeamEntry {
        canonical: "RCD Espanyol de Barcelona",
        clubelo: "Espanyol",
        aliases: &["Espanyol", "Espanol", "RCD Espanyol"],
    },
    TeamEntry {
        canonical: "FC Barcelona",
        clubelo: "Barcelona",
        aliases: &["Barcelona"],
    },
    TeamEntry {
        canonical: "Getafe CF",
        clubelo: "Getafe",
        aliases: &["Getafe"],
    },
    TeamEntry {
        canonical: "Girona FC",
        clubelo: "Girona",
        aliases: &["Girona"],
    },
    TeamEntry {
        canonical: "Granada CF",
        clubelo: "Granada",
        aliases: &["Granada"],
    },
    TeamEntry {
        canonical: "UD Las Palmas",
        clubelo: "LasPalmas",
        aliases: &["Las Palmas"],
    },
    TeamEntry {
        canonical: "CD Leganés",
        clubelo: "Leganes",
        aliases: &["Leganes", "Leganés"],
    },
    TeamEntry {
        canonical: "Levante UD",
        clubelo: "Levante",
        aliases: &["Levante"],
    },
    TeamEntry {
        canonical: "RCD Mallorca",
        clubelo: "Mallorca",
        aliases: &["Mallorca"],
    },
    TeamEntry {
        canonical: "Rayo Vallecano de Madrid",
        clubelo: "RayoVallecano",
        aliases: &["Rayo Vallecano", "Vallecano"],
    },
    TeamEntry {
        canonical: "Real Betis Balompié",
        clubelo: "Betis",
        aliases: &["Real Betis", "Betis"],
    },
    TeamEntry {
        canonical: "Real Madrid CF",
        clubelo: "realmadrid",
        aliases: &["Real Madrid"],
    },
    TeamEntry {
        canonical: "Real Sociedad de Fútbol",
        clubelo: "Sociedad",
        aliases: &["Real Sociedad", "Sociedad"],
    },
    TeamEntry {
        canonical: "Real Valladolid CF",
        clubelo: "Valladolid",
        aliases: &["Real Valladolid", "Valladolid"],
    },
    TeamEntry {
        canonical: "Sevilla FC",
        clubelo: "Sevilla",
        aliases: &["Sevilla"],
    },
    TeamEntry {
        canonical: "Valencia CF",
        clubelo: "Valencia",
        aliases: &["Valencia"],
    },
    TeamEntry {
        canonical: "Villarreal CF",
        clubelo: "Villarreal",
        aliases: &["Villarreal"],
    },
    TeamEntry {
        canonical: "SD Eibar",
        clubelo: "Eibar",
        aliases: &["Eibar"],
    },
    TeamEntry {
        canonical: "SD Huesca",
        clubelo: "Huesca",
        aliases: &["SD Huesca", "Huesca"],
    },
    TeamEntry {
        canonical: "Cádiz CF",
        clubelo: "Cadiz",
        aliases: &["Cadiz", "Cádiz"],
    },
    TeamEntry {
        canonical: "UD Almería",
        clubelo: "Almeria",
        aliases: &["Almeria", "Almería"],
    },
    TeamEntry {
        canonical: "RC Deportivo de La Coruña",
        clubelo: "Depor",
        aliases: &[
            "RC Deportivo La Coruña",
            "RC Deportivo La Coruna",
            "Deportivo La Coruna",
            "Deportivo La Coruña",
            "La Coruna",
            "Depor",
        ],
    },
    TeamEntry {
        canonical: "Real Racing Club de Santander",
        clubelo: "Santander",
        aliases: &["Santander", "Racing Santander"],
    },
    TeamEntry {
        canonical: "Málaga CF",
        clubelo: "Malaga",
        aliases: &["Malaga", "Málaga"],
    },
    TeamEntry {
        canonical: "Real Sporting de Gijón",
        clubelo: "Gijon",
        aliases: &[
            "Sporting Gijon",
            "Sporting Gijón",
            "Sp Gijon",
            "Gijon",
            "Gijón",
        ],
    },
    TeamEntry {
        canonical: "Real Oviedo",
        clubelo: "Oviedo",
        aliases: &["Oviedo"],
    },
    TeamEntry {
        canonical: "Córdoba CF",
        clubelo: "Cordoba",
        aliases: &["Cordoba", "Córdoba"],
    },
    // --- Bundesliga (BL1 / GER) ---
    TeamEntry {
        canonical: "FC Augsburg",
        clubelo: "Augsburg",
        aliases: &["Augsburg"],
    },
    TeamEntry {
        canonical: "Bayer 04 Leverkusen",
        clubelo: "Leverkusen",
        aliases: &["Bayer Leverkusen", "Leverkusen"],
    },
    TeamEntry {
        canonical: "FC Bayern München",
        clubelo: "Bayern",
        aliases: &["Bayern Munich", "Bayern München", "Bayern"],
    },
    TeamEntry {
        canonical: "VfL Bochum 1848",
        clubelo: "Bochum",
        aliases: &["Bochum", "VfL Bochum"],
    },
    TeamEntry {
        canonical: "Borussia Dortmund",
        clubelo: "Dortmund",
        aliases: &["Dortmund"],
    },
    TeamEntry {
        canonical: "Borussia Mönchengladbach",
        clubelo: "Gladbach",
        aliases: &[
            "Borussia Monchengladbach",
            "M'gladbach",
            "Gladbach",
            "Borussia M.Gladbach",
        ],
    },
    TeamEntry {
        canonical: "SV Darmstadt 98",
        clubelo: "Darmstadt",
        aliases: &["Darmstadt", "Darmstadt 98"],
    },
    TeamEntry {
        canonical: "Eintracht Frankfurt",
        clubelo: "Frankfurt",
        aliases: &["Frankfurt", "Ein Frankfurt"],
    },
    TeamEntry {
        canonical: "SV 07 Elversberg",
        clubelo: "Elversberg",
        aliases: &["Elversberg"],
    },
    TeamEntry {
        canonical: "SC Freiburg",
        clubelo: "Freiburg",
        aliases: &["Freiburg"],
    },
    TeamEntry {
        canonical: "Hamburger SV",
        clubelo: "Hamburg",
        aliases: &["Hamburg", "Hamburger SV"],
    },
    TeamEntry {
        canonical: "Hannover 96",
        clubelo: "Hannover",
        aliases: &["Hannover", "Hannover 96"],
    },
    TeamEntry {
        canonical: "1. FC Heidenheim 1846",
        clubelo: "Heidenheim",
        aliases: &["FC Heidenheim", "Heidenheim", "1. FC Heidenheim"],
    },
    TeamEntry {
        canonical: "Hertha BSC",
        clubelo: "Hertha",
        aliases: &["Hertha Berlin", "Hertha"],
    },
    TeamEntry {
        canonical: "TSG 1899 Hoffenheim",
        clubelo: "Hoffenheim",
        aliases: &["Hoffenheim", "TSG Hoffenheim"],
    },
    TeamEntry {
        canonical: "Holstein Kiel",
        clubelo: "Holstein",
        aliases: &["Holstein Kiel","HolsteinKiel", "Kiel"],
    },
    TeamEntry {
        canonical: "1. FC Köln",
        clubelo: "Koeln",
        aliases: &["FC Koln", "FC Köln", "1. FC Koln", "Koeln", "Koln", "Köln"],
    },
    TeamEntry {
        canonical: "RB Leipzig",
        clubelo: "RBLeipzig",
        aliases: &["RasenBallsport Leipzig", "Leipzig"],
    },
    TeamEntry {
        canonical: "1. FSV Mainz 05",
        clubelo: "Mainz",
        aliases: &["Mainz 05", "Mainz"],
    },
    TeamEntry {
        canonical: "1. FC Nürnberg",
        clubelo: "Nuernberg",
        aliases: &["Nuernberg", "Nurnberg", "Nuremberg", "Nürnberg"],
    },
    TeamEntry {
        canonical: "SC Paderborn 07",
        clubelo: "Paderborn",
        aliases: &["Paderborn", "SC Paderborn"],
    },
    TeamEntry {
        canonical: "FC Schalke 04",
        clubelo: "Schalke",
        aliases: &["Schalke 04", "Schalke"],
    },
    TeamEntry {
        canonical: "FC St. Pauli 1910",
        clubelo: "StPauli",
        aliases: &["St. Pauli", "St Pauli", "FC St. Pauli"],
    },
    TeamEntry {
        canonical: "VfB Stuttgart",
        clubelo: "Stuttgart",
        aliases: &["Stuttgart"],
    },
    TeamEntry {
        canonical: "1. FC Union Berlin",
        clubelo: "UnionBerlin",
        aliases: &["Union Berlin"],
    },
    TeamEntry {
        canonical: "SV Werder Bremen",
        clubelo: "Werder",
        aliases: &["Werder Bremen", "Werder"],
    },
    TeamEntry {
        canonical: "VfL Wolfsburg",
        clubelo: "Wolfsburg",
        aliases: &["Wolfsburg"],
    },
    TeamEntry {
        canonical: "DSC Arminia Bielefeld",
        clubelo: "Bielefeld",
        aliases: &["Arminia Bielefeld", "Bielefeld"],
    },
    TeamEntry {
        canonical: "SpVgg Greuther Fürth",
        clubelo: "Fuerth",
        aliases: &[
            "Greuther Fuerth",
            "Greuther Fürth",
            "Greuther Furth",
            "Fuerth",
            "Furth",
            "Fürth",
        ],
    },
    TeamEntry {
        canonical: "Fortuna Düsseldorf",
        clubelo: "Duesseldorf",
        aliases: &[
            "Fortuna Duesseldorf",
            "Fortuna Dusseldorf",
            "Duesseldorf",
            "Dusseldorf",
            "Düsseldorf",
        ],
    },
    TeamEntry {
        canonical: "FC Ingolstadt 04",
        clubelo: "Ingolstadt",
        aliases: &["Ingolstadt"],
    },
    // --- Serie A (SA / ITA) ---
    TeamEntry {
        canonical: "Atalanta BC",
        clubelo: "atalanta",
        aliases: &["Atalanta"],
    },
    TeamEntry {
        canonical: "Bologna FC 1909",
        clubelo: "Bologna",
        aliases: &["Bologna"],
    },
    TeamEntry {
        canonical: "Cagliari Calcio",
        clubelo: "Cagliari",
        aliases: &["Cagliari"],
    },
    TeamEntry {
        canonical: "Como 1907",
        clubelo: "Como",
        aliases: &["Como"],
    },
    TeamEntry {
        canonical: "Empoli FC",
        clubelo: "Empoli",
        aliases: &["Empoli"],
    },
    TeamEntry {
        canonical: "ACF Fiorentina",
        clubelo: "Fiorentina",
        aliases: &["Fiorentina"],
    },
    TeamEntry {
        canonical: "Frosinone Calcio",
        clubelo: "Frosinone",
        aliases: &["Frosinone"],
    },
    TeamEntry {
        canonical: "Genoa CFC",
        clubelo: "Genoa",
        aliases: &["Genoa"],
    },
    TeamEntry {
        canonical: "Hellas Verona FC",
        clubelo: "Verona",
        aliases: &["Verona", "Hellas Verona"],
    },
    TeamEntry {
        canonical: "FC Internazionale Milano",
        clubelo: "Inter",
        aliases: &["Inter", "Inter Milan", "Internazionale"],
    },
    TeamEntry {
        canonical: "Juventus FC",
        clubelo: "Juventus",
        aliases: &["Juventus"],
    },
    TeamEntry {
        canonical: "SS Lazio",
        clubelo: "Lazio",
        aliases: &["Lazio"],
    },
    TeamEntry {
        canonical: "US Lecce",
        clubelo: "Lecce",
        aliases: &["Lecce"],
    },
    TeamEntry {
        canonical: "AC Milan",
        clubelo: "Milan",
        aliases: &["Milan"],
    },
    TeamEntry {
        canonical: "AC Monza",
        clubelo: "Monza",
        aliases: &["Monza"],
    },
    TeamEntry {
        canonical: "SSC Napoli",
        clubelo: "Napoli",
        aliases: &["Napoli"],
    },
    TeamEntry {
        canonical: "Parma Calcio 1913",
        clubelo: "parma-calcio-1913",
        aliases: &["Parma", "Parma Calcio"],
    },
    TeamEntry {
        canonical: "AS Roma",
        clubelo: "Roma",
        aliases: &["Roma"],
    },
    TeamEntry {
        canonical: "US Salernitana 1919",
        clubelo: "us-salernitana-1919",
        aliases: &["Salernitana"],
    },
    TeamEntry {
        canonical: "UC Sampdoria",
        clubelo: "Sampdoria",
        aliases: &["Sampdoria"],
    },
    TeamEntry {
        canonical: "US Sassuolo Calcio",
        clubelo: "Sassuolo",
        aliases: &["Sassuolo"],
    },
    TeamEntry {
        canonical: "Spezia Calcio",
        clubelo: "Spezia",
        aliases: &["Spezia"],
    },
    TeamEntry {
        canonical: "Torino FC",
        clubelo: "Torino",
        aliases: &["Torino"],
    },
    TeamEntry {
        canonical: "Udinese Calcio",
        clubelo: "Udinese",
        aliases: &["Udinese"],
    },
    TeamEntry {
        canonical: "Venezia FC",
        clubelo: "venezia-fc",
        aliases: &["Venezia"],
    },
    TeamEntry {
        canonical: "Benevento Calcio",
        clubelo: "Benevento",
        aliases: &["Benevento"],
    },
    TeamEntry {
        canonical: "Brescia Calcio",
        clubelo: "Brescia",
        aliases: &["Brescia"],
    },
    TeamEntry {
        canonical: "Carpi FC 1909",
        clubelo: "Carpi",
        aliases: &["Carpi"],
    },
    TeamEntry {
        canonical: "AC ChievoVerona",
        clubelo: "Chievo",
        aliases: &["Chievo", "Chievo Verona"],
    },
    TeamEntry {
        canonical: "FC Crotone",
        clubelo: "Crotone",
        aliases: &["Crotone"],
    },
    TeamEntry {
        canonical: "Delfino Pescara 1936",
        clubelo: "Pescara",
        aliases: &["Pescara"],
    },
    TeamEntry {
        canonical: "US Cremonese",
        clubelo: "Cremonese",
        aliases: &["Cremonese"],
    },
    TeamEntry {
        canonical: "SPAL",
        clubelo: "Spal",
        aliases: &["SPAL 2013", "Spal"],
    },
    TeamEntry {
        canonical: "Palermo FC",
        clubelo: "Palermo",
        aliases: &["Palermo"],
    },
    TeamEntry {
        canonical: "Pisa SC",
        clubelo: "pisa-sc",
        aliases: &["Pisa"],
    },
    // --- Ligue 1 (FL1 / FRA) ---
    TeamEntry {
        canonical: "Angers SCO",
        clubelo: "Angers",
        aliases: &["Angers"],
    },
    TeamEntry {
        canonical: "AJ Auxerre",
        clubelo: "Auxerre",
        aliases: &["Auxerre"],
    },
    TeamEntry {
        canonical: "Stade Brestois 29",
        clubelo: "Brest",
        aliases: &["Brest", "Stade Brestois"],
    },
    TeamEntry {
        canonical: "Clermont Foot 63",
        clubelo: "Clermont",
        aliases: &["Clermont Foot", "Clermont"],
    },
    TeamEntry {
        canonical: "Le Havre AC",
        clubelo: "LeHavre",
        aliases: &["Le Havre"],
    },
    TeamEntry {
        canonical: "Le Mans FC",
        clubelo: "LeMans",
        aliases: &["Le Mans"],
    },
    TeamEntry {
        canonical: "RC Lens",
        clubelo: "Lens",
        aliases: &["Lens", "Racing Club de Lens"],
    },
    TeamEntry {
        canonical: "LOSC Lille",
        clubelo: "Lille",
        aliases: &["Lille", "Lille OSC"],
    },
    TeamEntry {
        canonical: "FC Lorient",
        clubelo: "Lorient",
        aliases: &["Lorient"],
    },
    TeamEntry {
        canonical: "Olympique Lyonnais",
        clubelo: "Lyon",
        aliases: &["Lyon"],
    },
    TeamEntry {
        canonical: "Olympique de Marseille",
        clubelo: "Marseille",
        aliases: &["Marseille"],
    },
    TeamEntry {
        canonical: "FC Metz",
        clubelo: "Metz",
        aliases: &["Metz"],
    },
    TeamEntry {
        canonical: "AS Monaco FC",
        clubelo: "Monaco",
        aliases: &["Monaco", "AS Monaco"],
    },
    TeamEntry {
        canonical: "Montpellier HSC",
        clubelo: "Montpellier",
        aliases: &["Montpellier"],
    },
    TeamEntry {
        canonical: "FC Nantes",
        clubelo: "Nantes",
        aliases: &["Nantes"],
    },
    TeamEntry {
        canonical: "OGC Nice",
        clubelo: "Nice",
        aliases: &["Nice"],
    },
    TeamEntry {
        canonical: "Paris Saint-Germain FC",
        clubelo: "ParisSG",
        aliases: &[
            "Paris Saint Germain",
            "Paris Saint-Germain",
            "Paris SG",
            "PSG",
        ],
    },
    TeamEntry {
        canonical: "Stade de Reims",
        clubelo: "Reims",
        aliases: &["Reims"],
    },
    TeamEntry {
        canonical: "Stade Rennais FC 1901",
        clubelo: "Rennes",
        aliases: &["Rennes", "Stade Rennais"],
    },
    TeamEntry {
        canonical: "AS Saint-Étienne",
        clubelo: "SaintEtienne",
        aliases: &[
            "Saint-Etienne",
            "Saint Etienne",
            "Saint-Étienne",
            "St Etienne",
        ],
    },
    TeamEntry {
        canonical: "RC Strasbourg Alsace",
        clubelo: "Strasbourg",
        aliases: &["Strasbourg"],
    },
    TeamEntry {
        canonical: "Toulouse FC",
        clubelo: "Toulouse",
        aliases: &["Toulouse"],
    },
    TeamEntry {
        canonical: "ESTAC Troyes",
        clubelo: "Troyes",
        aliases: &["Troyes", "ES Troyes AC"],
    },
    TeamEntry {
        canonical: "AC Ajaccio",
        clubelo: "Ajaccio",
        aliases: &["Ajaccio"],
    },
    TeamEntry {
        canonical: "Amiens SC",
        clubelo: "Amiens",
        aliases: &["Amiens"],
    },
    TeamEntry {
        canonical: "SC Bastia",
        clubelo: "Bastia",
        aliases: &["Bastia"],
    },
    TeamEntry {
        canonical: "FC Girondins de Bordeaux",
        clubelo: "Bordeaux",
        aliases: &["Bordeaux"],
    },
    TeamEntry {
        canonical: "SM Caen",
        clubelo: "Caen",
        aliases: &["Caen"],
    },
    TeamEntry {
        canonical: "Dijon FCO",
        clubelo: "Dijon",
        aliases: &["Dijon"],
    },
    TeamEntry {
        canonical: "Evian TG FC",
        clubelo: "Evian",
        aliases: &["Evian Thonon Gaillard", "Evian"],
    },
    TeamEntry {
        canonical: "En Avant Guingamp",
        clubelo: "Guingamp",
        aliases: &["Guingamp"],
    },
    TeamEntry {
        canonical: "AS Nancy-Lorraine",
        clubelo: "Nancy",
        aliases: &["Nancy", "AS Nancy"],
    },
    TeamEntry {
        canonical: "Nîmes Olympique",
        clubelo: "Nimes",
        aliases: &["Nimes", "Nîmes"],
    },
    TeamEntry {
        canonical: "Paris FC",
        clubelo: "ParisFC",
        aliases: &["Paris FC"],
    },
];

/// Porównuje nazwy bez rozróżniania wielkości liter oraz spacji/kropek/myślników
/// (np. `"Man City"` z CSV ClubElo pasuje do sluga `"ManCity"`).
fn compact_ascii_eq(a: &str, b: &str) -> bool {
    let mut ia = a
        .chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_lowercase());
    let mut ib = b
        .chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_lowercase());
    loop {
        match (ia.next(), ib.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if x == y => continue,
            _ => return false,
        }
    }
}

fn find_team_entry(raw: &str) -> Option<&'static TeamEntry> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Najpierw szukamy dokładnego dopasowania bez uwzględniania wielkości liter,
    // żeby nie pomylić podobnie brzmiących nazw w różnych ligach.
    TEAMS
        .iter()
        .find(|entry| {
            entry.canonical.eq_ignore_ascii_case(trimmed)
                || entry.clubelo.eq_ignore_ascii_case(trimmed)
                || entry
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(trimmed))
        })
        .or_else(|| {
            TEAMS.iter().find(|entry| {
                compact_ascii_eq(entry.canonical, trimmed)
                    || compact_ascii_eq(entry.clubelo, trimmed)
                    || entry
                        .aliases
                        .iter()
                        .any(|alias| compact_ascii_eq(alias, trimmed))
            })
        })
}

/// Ujednolica nazwę drużyny z Understat, ClubElo oraz Football-Data.org do wspólnej postaci kanonicznej.
pub fn normalize_football_team_name(raw: &str) -> &str {
    find_team_entry(raw)
        .map(|entry| entry.canonical)
        .unwrap_or_else(|| raw.trim())
}

/// Zwraca slug klubu wymagany przez `http://api.clubelo.com/{slug}`.
pub fn clubelo_slug(name: &str) -> Option<&'static str> {
    find_team_entry(name).map(|entry| entry.clubelo)
}
