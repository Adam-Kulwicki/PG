"""Podstawowe obliczenia Elo dla meczów Counter-Strike."""

from collections.abc import Iterable
from typing import Optional


# Nowa drużyna zaczyna od 1500 punktów.
INITIAL_RATING = 1500.0

# Przewaga 400 punktów Elo daje prawdopodobieństwo wygranej równe 10/11.
ELO_SCALE = 400.0

# Współczynnik K określa, jak mocno pojedynczy wynik wpływa na rating.
K_FACTOR = 32.0

# Im większa przewaga map, tym większa zmiana ratingu po wyniku.
SCORE_MARGIN_WEIGHT = 0.5

# Jednostka średniego odchylenia od prognoz odpowiada 50 punktom Elo.
FORM_ELO_WEIGHT = 50.0


def decay_rating(rating: float, idle_days: float, half_life_days: float) -> float:
    """Przybliża rating do 1500 po nieujemnym czasie i przy dodatnim okresie półtrwania."""
    # Po jednym okresie półtrwania zostaje połowa odchylenia od 1500.
    retained_fraction = 0.5 ** (idle_days / half_life_days)
    return INITIAL_RATING + (rating - INITIAL_RATING) * retained_fraction


def win_probability(team_rating: float, opponent_rating: float) -> float:
    """Zwraca prawdopodobieństwo wygranej pierwszej drużyny w skali od 0 do 1.

    Drużyny o równych ratingach mają po 50% szans.
    """
    rating_difference = opponent_rating - team_rating
    return 1.0 / (1.0 + 10.0 ** (rating_difference / ELO_SCALE))


def form_rating_bonus(residuals: Iterable[float], window: int) -> float:
    """Przelicza różnice wyniku i prognozy z ostatnich meczów na korektę Elo."""
    # Brakujące mecze w oknie wnoszą zero, więc jeden wynik ma ograniczony wpływ.
    return FORM_ELO_WEIGHT * sum(residuals) / window


def update_ratings(
    team_rating: float,
    opponent_rating: float,
    team_won: bool,
    *,
    map_score: Optional[tuple[int, int]] = None,
) -> tuple[float, float]:
    """Zwraca nowe ratingi w kolejności drużyna, przeciwnik.

    team_won to True po wygranej pierwszej drużyny i False po jej przegranej.
    map_score zawiera liczby wygranych map w tej samej kolejności.
    """
    expected_score = win_probability(team_rating, opponent_rating)
    actual_score = 1.0 if team_won else 0.0

    margin_multiplier = 1.0
    if map_score is not None:
        team_maps, opponent_maps = map_score
        if min(map_score) < 0 or team_maps == opponent_maps:
            raise ValueError("Wynik map musi mieć nieujemne wartości i rozstrzygnięcie")
        if (team_maps > opponent_maps) != team_won:
            raise ValueError("Wynik map musi być zgodny ze zwycięzcą")

        # Jedna mapa przewagi daje zwykłą aktualizację. W BO3 wynik 2:0 daje mnożnik 1,25.
        extra_margin = (abs(team_maps - opponent_maps) - 1) / max(map_score)
        margin_multiplier += SCORE_MARGIN_WEIGHT * extra_margin

    # Im bardziej zaskakujący wynik, tym więcej punktów przechodzi do zwycięzcy.
    rating_change = K_FACTOR * (actual_score - expected_score) * margin_multiplier
    return team_rating + rating_change, opponent_rating - rating_change
