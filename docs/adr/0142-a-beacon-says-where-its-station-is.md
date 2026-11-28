# ADR-0142: A beacon says where its station is, as precisely as its operator chose

**Status:** accepted, 2026-11-28 (RadioPost's design 07, the author's decisions 8, 9 and 10). The
beacon body: **bytes after the capability byte, no version change**; by the author's reading not
an air-protocol change for P5-7's four weeks.

## 1. Context

The author asked that a beacon carry the station's position when one is known (GPS or
otherwise): a listener then sees how far and which way a station heard is, and RadioPost's client
predicts a band's chance for that path (its design §7). A 6-character grid square is the
amateur's usual figure; a portable or mobile station may want its exact position, and a home
station may want to say nothing.

## 2. Decision

### 2.1 The bytes

After the capability byte, a precision byte and the position (air-interface §7.4):

| Precision | Bytes | Position |
|---|---|---|
| none (absent) | 0 | nothing said |
| 1 | 2 | the 4-character square: indices across and up, 0–179, one byte each |
| 2 | 4 | the 6-character square: indices 0–4319 across and up, as the word `x << 13 \| y` |
| 3 | 6 | latitude and longitude, 24 bits each: about 1.2 m north and 2.4 m east at the equator |

The body is 15 bytes at most and the beacon stays one tone-24 frame of 5.4 s. A precision a
receiver does not know is no position; the bytes before it still read. A square is heard as its
centre, worked in whole 24ths and 48ths of a degree (`grid::square_centre`), so that RadioPost's
programs, reading the same square, get the same two numbers to the last bit.

### 2.2 Where it comes from

- A program's `beacon` gives `position` `{lat, lon, precision}` (`grid4`, `grid6` or `exact`);
  Aether sends as much as the precision says, the square worked from the position
  (`grid::square_of`). `position: null` says nothing.
- A beacon given none (the panel's, **Repeat**'s, a host program's `CQFRAME`) carries
  `[operator] grid` when one is set: 4 characters as a 4-character square, 6 or 8 as a
  6-character one.

The `frame` event, the heard list (`heard.json`'s `position`: `lat`, `lon`, `precision`, `grid`
for a square), `heard.list` and the `heard` event carry it.

## 3. Alternatives considered

- **A grid as text.** Six bytes for a 6-character square against four, and no exact position.
- **APRS's compressed position.** Thirteen bytes with symbol and course, designed for another
  frame; Aether needs the position alone.
- **Exact always.** The author chose exact by default in RadioPost, with the operator able to say
  less; the modem sends what it is given and never more.

## 4. Consequences

- Everyone listening hears the position, as with any amateur transmission; RadioPost lets its
  operator choose what goes (its manual says so).
- `[operator] grid`, until now for the field log's sidecars, is now on the air in every beacon
  that is given no position: Setup's tooltip says so.
- An earlier Aether reads the callsign and the bandwidth and ignores the rest.
- Tests: the aether-link beacon body tests (each precision round trips, the worked bytes, an
  unknown precision), `grid::a_square_is_found_and_centred_as_radiopost_finds_it`, and the
  station test of ADR-0139.
