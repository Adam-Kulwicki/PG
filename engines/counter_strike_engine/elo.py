"""Podstawowe obliczenia Elo dla meczów Counter-Strike."""


# Przewaga 400 punktów Elo daje prawdopodobieństwo wygranej równe 10/11.
ELO_SCALE = 400.0

# Współczynnik K określa, jak mocno pojedynczy wynik wpływa na rating.
K_FACTOR = 32.0


def win_probability(team_rating: float, opponent_rating: float) -> float:
    """Zwraca prawdopodobieństwo wygranej pierwszej drużyny w skali od 0 do 1.

    Drużyny o równych ratingach mają po 50% szans.
    """
    rating_difference = opponent_rating - team_rating
    return 1.0 / (1.0 + 10.0 ** (rating_difference / ELO_SCALE))


def update_ratings(
    team_rating: float, opponent_rating: float, team_won: bool
) -> tuple[float, float]:
    """Zwraca nowe ratingi w kolejności drużyna, przeciwnik.

    team_won to True po wygranej pierwszej drużyny i False po jej przegranej.
    """
    expected_score = win_probability(team_rating, opponent_rating)
    actual_score = 1.0 if team_won else 0.0

    # Im bardziej zaskakujący wynik, tym więcej punktów przechodzi do zwycięzcy.
    rating_change = K_FACTOR * (actual_score - expected_score)
    return team_rating + rating_change, opponent_rating - rating_change
