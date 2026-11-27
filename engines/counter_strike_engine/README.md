# CS Prediction Engine

Silnik predykcyjny meczów Counter-Strike 2 oparty na systemie rankingowym Elo. Projekt pozwala pobierać historię zakończonych meczów z backendu PG, odtwarzać ją z lokalnego pliku CSV, obliczać prawdopodobieństwo zwycięstwa drużyn oraz porównywać skuteczność różnych wariantów modelu.

## Zaimplementowane funkcjonalności

- **Import danych historycznych** – pobieranie zakończonych meczów CS2 przez REST API backendu PG, z paginacją, walidacją i zapisem do CSV.
- **Klasyczne Elo** – każda nowa drużyna zaczyna z ratingiem 1500. Rating jest aktualizowany po udostępnieniu wyniku meczu (K = 32).
- **Rating decay** – podczas nieaktywności rating drużyny stopniowo zbliża się do 1500. Okres półtrwania jest konfigurowalny.
- **Score margin** – przewaga w liczbie wygranych map wpływa na wielkość zmiany ratingu, np. wynik 2:0 powoduje o 25% większą zmianę niż 2:1 przy tych samych ratingach i zwycięzcy.
- **Forma drużyny** – korekta prognozy na podstawie ostatnich N meczów i różnicy pomiędzy rzeczywistymi wynikami a oczekiwaniami klasycznego Elo.
- **Backtesting** – chronologiczne odtwarzanie historii. Prognozy powstają w momencie rozpoczęcia meczu, a ratingi są aktualizowane dopiero po udostępnieniu wyniku, co ogranicza ryzyko wykorzystania przyszłych wyników. Gdy czas dostępności wyniku nie jest znany, symulacja korzysta z jawnie oznaczonego przybliżenia.
- **Ocena modeli** – porównanie klasycznego Elo, Elo z zanikiem ratingu, prognozy 50/50 i opcjonalnie wariantu rozszerzonego o formę lub przewagę map.
- **Raportowanie** – zapis prognoz do CSV oraz wyświetlanie metryk Brier score i log loss w terminalu.

## Struktura modułu

| Plik | Opis |
| --- | --- |
| `dataloader.py` | Pobieranie i walidacja historii meczów z API backendu PG oraz eksport do CSV. |
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

Identyfikator meczu musi być unikalny, daty muszą zawierać strefę czasową, a wynik musi być dostępny po rozpoczęciu meczu. Kolumny `maps_won_a` i `maps_won_b` są opcjonalne; umożliwiają użycie mechanizmu score margin. Dataloader zapisuje dodatkowo `season`, `game_version` i `result_time_source`. Ostatnia kolumna wskazuje, czy czas dostępności wyniku pochodzi z API (`reported`), czy jest szacowany (`estimated`).

## Uruchamianie

Wszystkie poniższe polecenia należy uruchamiać z katalogu `engines/` w repozytorium PG. Dataloader wymaga dostępu do działającego backendu zawierającego mecze CS2; replay i report działają offline na pobranym CSV.

### Pobranie historii meczów

```bash
python3 -m counter_strike_engine.dataloader \
    --api-url http://localhost:3000 \
    --output counter_strike_engine/data/history_test.csv \
    --result-delay-hours 3
```

Adres `--api-url` można zastąpić adresem zdalnego backendu PG zawierającego dane CS2. Nie trzeba uruchamiać lokalnego Dockera, jeśli korzysta się ze zdalnego backendu. W przypadku lokalnej instancji bez danych CS2 może być potrzebna konfiguracja importera, w tym klucza `PANDASCORE_API_KEY`.

Dataloader pobiera strony po maksymalnie 500 rekordów z endpointu `/api/v1/matches` z filtrami `sport=CS2` i `status=FINISHED`. Eksport obejmuje mecze z rozpoznanym zwycięzcą. Brak danych lub niepoprawne rekordy objęte walidacją powodują zakończenie z błędem bez zastąpienia dotychczasowego pliku.

| Argument | Opis |
| --- | --- |
| `--api-url` | Bazowy adres backendu (domyślnie `http://localhost:3000`). |
| `--output` | Ścieżka do wynikowego CSV (wymagana). |
| `--result-delay-hours` | Szacowany czas od rozpoczęcia meczu do dostępności wyniku, używany tylko przy braku tej informacji w API. |

Wartość 3 godzin jest założeniem testowym, a nie zmierzonym czasem trwania wszystkich meczów.

### Klasyczne Elo

```bash
python3 -m counter_strike_engine.replay counter_strike_engine/data/history_test.csv
```

### Elo z zanikiem ratingu, przewagą map i formą

```bash
python3 -m counter_strike_engine.replay counter_strike_engine/data/history_test.csv \
    --half-life-days 90 --score-margin --form-window 5
```

Program wyświetla prognozy dla kolejnych meczów, faktycznych zwycięzców i końcowy ranking drużyn.

### Porównanie modeli i zapis raportu

```bash
python3 -m counter_strike_engine.report counter_strike_engine/data/history_test.csv \
    --test-from 2026-01-01T00:00:00Z \
    --half-life-days 90 --score-margin --form-window 5 \
    --output counter_strike_engine/data/predictions.csv
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

## Wyniki pierwszego backtestu na rzeczywistych danych

W teście integracyjnym pobrano 5832 zakończone mecze CS2 ze zdalnego backendu PG. Raport objął 2057 meczów rozpoczynających się od 1 stycznia 2026 r. (UTC). Wcześniejsze mecze posłużyły do inicjalizacji ratingów, a w okresie oceny ratingi były nadal aktualizowane po dostępności kolejnych wyników (backtest kroczący).

Parametry: K = 32, rating początkowy = 1500, okres półtrwania = 90 dni, okno formy N = 5, szacowane opóźnienie wyniku = 3 godziny (jeśli brak czasu z API).

| Model | Brier score ↓ | Log loss ↓ |
| --- | ---: | ---: |
| **Klasyczne Elo** | **0.234774** | **0.661935** |
| Elo z zanikiem (90 dni) | 0.240964 | 0.674519 |
| 50/50 | 0.250000 | 0.693147 |
| Elo + zanik + wynik map + forma N=5 | 0.241512 | 0.675473 |

Klasyczne Elo osiągnęło najlepsze wyniki w obu metrykach: redukcję Brier score o 6,09% oraz log loss o 4,50% względem prognozy 50/50. Wszystkie warianty Elo przewyższyły punkt odniesienia, ale rozszerzenia w testowanej konfiguracji nie poprawiły klasycznego Elo.

Wyniki są wstępne. Nie pozwalają ocenić osobnego wpływu formy i przewagi map, ponieważ raport testuje je łącznie z zanikiem. Nie zweryfikowano udziału szacowanych czasów dostępności wyników ani wpływu założonego opóźnienia na ocenę. Metryki nie określają bezpośrednio odsetka poprawnie wytypowanych zwycięzców ani rentowności predykcji.

## Aktualny stan projektu

Silnik badawczy działa na rzeczywistych danych z API backendu PG. Lokalny CSV umożliwia powtarzalne eksperymenty bez każdorazowego pobierania historii. Replay buduje ratingi w pamięci i wyświetla końcowy ranking, a report zapisuje prognozy historyczne oraz wyświetla metryki. Stan wytrenowanego modelu nie jest jeszcze trwale zapisywany. Nie zaimplementowano automatycznych predykcji dla przyszłych spotkań ani produkcyjnego agenta.

Wygenerowanych plików z historią i predykcjami nie należy domyślnie dodawać do repozytorium. W katalogu silnika można uwzględnić w `.gitignore` regułę `data/*.csv` (z zachowaniem ewentualnych celowo wersjonowanych danych testowych).

## Dalszy rozwój

- Ocena jakości danych, w tym udziału szacowanych czasów wyników i wrażliwości metryk na przyjęte opóźnienie.
- Porównanie różnych wartości K, okresów półtrwania i niezależnego wpływu formy oraz przewagi map.
- Dobór parametrów na wcześniejszym okresie walidacyjnym i końcowa ocena na osobnym, późniejszym okresie testowym.
- Dodanie testów automatycznych, metryki accuracy i oceny kalibracji prawdopodobieństw.
- Wydzielenie wspólnego stanu modelu dla backtestingu i pracy produkcyjnej oraz jego zapis i odczyt (ratingi, daty ostatnich aktualizacji, forma, parametry i informacje o przetworzonych wynikach).
- Generowanie predykcji dla nadchodzących meczów i integracja agenta z backendem zgodnie z jego rzeczywistym kontraktem API.