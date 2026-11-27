# CS Prediction Engine

Silnik predykcyjny meczów Counter-Strike oparty na systemie rankingowym Elo. Projekt pozwala odtwarzać historię meczów z lokalnego pliku CSV, obliczać prawdopodobieństwo zwycięstwa drużyn oraz porównywać skuteczność różnych wariantów modelu.

## Zaimplementowane funkcjonalności

- **Klasyczne Elo** – każda nowa drużyna zaczyna z ratingiem 1500. Rating jest aktualizowany po udostępnieniu wyniku meczu (K = 32).
- **Rating decay** – podczas nieaktywności rating drużyny stopniowo zbliża się do 1500. Okres półtrwania jest konfigurowalny.
- **Score margin** – przewaga w liczbie wygranych map wpływa na wielkość zmiany ratingu, np. wynik 2:0 powoduje o 25% większą zmianę niż 2:1 przy tych samych ratingach i zwycięzcy.
- **Forma drużyny** – korekta prognozy na podstawie ostatnich N meczów i różnicy pomiędzy rzeczywistymi wynikami a oczekiwaniami klasycznego Elo.
- **Backtesting** – chronologiczne odtwarzanie historii. Prognozy powstają w momencie rozpoczęcia meczu, a ratingi są aktualizowane dopiero po udostępnieniu wyniku, co zapobiega wykorzystaniu przyszłych wyników.
- **Ocena modeli** – porównanie klasycznego Elo, Elo z zanikiem ratingu, prognozy 50/50 i opcjonalnie wariantu rozszerzonego o formę lub przewagę map.
- **Raportowanie** – zapis prognoz do CSV oraz wyświetlanie metryk Brier score i log loss w terminalu.

## Struktura modułu

| Plik | Opis |
| --- | --- |
| `elo.py` | Obliczanie prawdopodobieństwa wygranej, aktualizacja ratingów, zanik ratingu i korekta formy. |
| `replay.py` | Wczytywanie i walidacja CSV, odtwarzanie historii, generowanie prognoz i końcowego rankingu. |
| `report.py` | Porównywanie wariantów modelu, obliczanie metryk i eksport raportu CSV. |

## Wymagania

- Python 3.9 lub nowszy.
- Brak dodatkowych bibliotek – moduły korzystają z biblioteki standardowej Pythona.

## Format danych wejściowych

Historia meczów powinna być zapisana w pliku CSV, np. `history.csv`:

```csv
match_id,starts_at,result_available_at,team_a_id,team_b_id,winner_team_id,maps_won_a,maps_won_b
1,2026-01-01T12:00:00Z,2026-01-01T14:00:00Z,team_a,team_b,team_a,2,0
2,2026-01-02T12:00:00Z,2026-01-02T14:00:00Z,team_a,team_b,team_b,1,2
```

Identyfikator meczu musi być unikalny, daty muszą zawierać strefę czasową, a wynik musi być dostępny po rozpoczęciu meczu. Kolumny `maps_won_a` i `maps_won_b` są opcjonalne; umożliwiają użycie mechanizmu score margin.

## Uruchamianie

Poniższe polecenia zakładają, że trzy moduły znajdują się w pakiecie `cs_prediction_engine`, a terminal jest otwarty w jego katalogu nadrzędnym. Jeżeli w repozytorium pakiet ma inną nazwę, należy ją podstawić w poleceniach.

### Klasyczne Elo

```bash
python3 -m cs_prediction_engine.replay history.csv
```

### Elo z zanikiem ratingu, przewagą map i formą

```bash
python3 -m cs_prediction_engine.replay history.csv \
    --half-life-days 90 --score-margin --form-window 5
```

Program wyświetla prognozy dla kolejnych meczów, faktycznych zwycięzców i końcowy ranking drużyn.

### Porównanie modeli i zapis raportu

```bash
python3 -m cs_prediction_engine.report history.csv \
    --test-from 2026-01-01T00:00:00Z \
    --half-life-days 90 --score-margin --form-window 5 \
    --output predictions.csv
```

| Argument | Opis |
| --- | --- |
| `--half-life-days` | Okres półtrwania odchylenia ratingu od 1500, w dniach. |
| `--score-margin` | Włącza uwzględnianie przewagi map przy aktualizacji Elo. |
| `--form-window` | Liczba ostatnich meczów uwzględnianych w formie (0 wyłącza). |
| `--test-from` | Początek okresu ocenianego w raporcie (z pełnej wcześniejszej historii budowane są ratingi). |
| `--output` | Ścieżka do wynikowego pliku CSV. |

## Ocena skuteczności

- **Brier score** – średni kwadrat różnicy pomiędzy przewidywanym prawdopodobieństwem wygranej a rzeczywistym wynikiem.
- **Log loss** – średnia wartość ujemnego logarytmu prawdopodobieństwa przypisanego faktycznemu zwycięzcy. Mocniej karze błędne prognozy o wysokiej pewności.

W obu metrykach niższy wynik oznacza lepszą jakość prognoz. Punktem odniesienia jest model przypisujący obu drużynom po 50% szans (Brier score = 0,25; log loss ≈ 0,6931).

## Aktualny stan projektu

Silnik działa na lokalnej historii meczów w formacie CSV. Automatyczne pobieranie danych z HLTV nie jest jeszcze zaimplementowane. Skuteczność predykcji na rzeczywistych danych wymaga osobnej oceny.
