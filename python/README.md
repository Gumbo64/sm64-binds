# sm64-binds

Mario 64 binds for Python.

This is the Python version of [sm64-binds](https://github.com/Gumbo64/sm64-binds)

To avoid shipping copyrighted ROM data, the WASM module is distributed XOR'd
against a US `.z64` ROM and recombined at runtime. The ROM is required: a US
`z64` copy, exactly 8.00 MB.

## Install

```
pip install sm64-binds
```

## Usage

```python
from sm64_binds import GamePad, SM64GameGenerator, buttons

generator = SM64GameGenerator.from_file("baserom.us.z64")
game = generator.create_game()

game.step_game(GamePad(buttons.A_BUTTON, 0, 0))

state = game.get_game_state()
print(state)              # position, level, lives, coins, health, ...
print(state.numStars)
```

`SM64GameGenerator` validates the ROM's SHA-1, recombines the payload and
compiles the WASM module once. Reuse it: `create_game` is then cheap.

### Cloning a game

Copy an instance to branch off the same state without replaying:

```python
clone = generator.copy(game)

# ... play the clone differently ...
generator.copy_to(game, clone)   # reuse an existing instance instead
```

A copy captures the two mutable regions of linear memory — the emscripten stack
and everything after the game's constant data (`generator.read_only`, read from
the WASM) — so a copied game continues exactly where its source was.

## API

| Export | Purpose |
| --- | --- |
| `SM64GameGenerator` | Validates a ROM, compiles the module, creates games. |
| `SM64Game` | `step_game`, `get_game_state`. |
| `GamePad` | `button`, `stick_x`, `stick_y`. |
| `GameState` | `numStars`, `pos`, `vel`, `lakituPos`, `levelNum`, `lives`, `coins`, `health`, ... |
| `buttons` | N64 button masks (`A_BUTTON`, `START_BUTTON`, `U_JPAD`, ...). |

## Licence

MIT. The game itself is © Nintendo; this package contains no ROM or game data.