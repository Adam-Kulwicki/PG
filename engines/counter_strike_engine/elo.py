"""Podstawowe obliczenia Elo dla meczów Counter-Strike."""


# Przewaga 400 punktów Elo daje prawdopodobieństwo wygranej równe 10/11.
ELO_SCALE = 400.0


def win_probability(team_rating: float, opponent_rating: float) -> float:
    """Zwraca prawdopodobieństwo wygranej pierwszej drużyny w skali od 0 do 1.

    Drużyny o równych ratingach mają po 50% szans.
    """
    rating_difference = opponent_rating - team_rating
    return 1.0 / (1.0 + 10.0 ** (rating_difference / ELO_SCALE))
