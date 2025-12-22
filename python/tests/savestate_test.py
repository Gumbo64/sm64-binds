import random

A_BUTTON = 0x8000
B_BUTTON = 0x4000
L_TRIG = 0x0020
R_TRIG = 0x0010
Z_TRIG = 0x2000
START_BUTTON = 0x1000
U_JPAD = 0x0800
L_JPAD = 0x0200
R_JPAD = 0x0100
D_JPAD = 0x0400
U_CBUTTONS = 0x0008
L_CBUTTONS = 0x0002
R_CBUTTONS = 0x0001
D_CBUTTONS = 0x0004


import struct

import numpy as np
import matplotlib.pyplot as plt
from mpl_toolkits.mplot3d import Axes3D
import imageio
import os


from gameplay import GAMEPLAY
from sm64_binds import SM64Game, SM64GameGenerator, GamePad


# -------------------------------------------------------------
# Extended GIF creator with two colors
# -------------------------------------------------------------
def create_3d_rotation_gif_two_colors(pts1, pts2, filename='clone_divergence.gif', points_per_frame=100):
    import matplotlib.pyplot as plt
    import imageio
    import os

    frames = []
    print(len(pts1), len(pts2))
    for i in range(0, len(pts1), points_per_frame):
        fig = plt.figure()
        ax = fig.add_subplot(111, projection='3d')

        # ORIGINAL path (GAMEPLAY): BLUE
        ax.scatter(
            pts1[:i, 0],
            pts1[:i, 2],
            pts1[:i, 1],
            c='blue',
            marker='o',
            s=0.4,
            label='original',
            alpha=0.5
        )

        # CLONE path (random): RED
        ax.scatter(
            pts2[:i, 0],
            pts2[:i, 2],
            pts2[:i, 1],
            c='red',
            marker='o',
            s=4,
            label='clone'
        )

        ax.set_xlim([-8000, 8000])
        ax.set_ylim([-8000, 8000])
        ax.set_zlim([-8000, 8000])

        ax.view_init(30, (i // points_per_frame) * 5 % 360)
        ax.legend()

        plt.savefig('temp.png')
        plt.close(fig)
        frames.append(imageio.imread('temp.png'))

    os.remove('temp.png')
    imageio.mimsave(filename, frames, fps=30)

    print(f"GIF saved: {filename}")


def run():
    # ----------------------------------
    # Load ROM and initialize game
    # ----------------------------------
    rom_bytes = open("./baserom.us.z64", "rb").read()

    generator = SM64GameGenerator.new(rom_bytes)
    game = generator.create_game()

    # ----------------------------------
    # gameplay loop
    # ----------------------------------
    positions_list = []

    positions_clone_list = []

    game2 = None

    for i in range(len(GAMEPLAY.keys()) // 4):
        # Extract controller bytes from GAMEPLAY dict
        b = bytes([GAMEPLAY[str(n)] for n in range(4*i, 4*i+4)])

        # Parse inputs
        button = (b[0] << 8) | b[1]
        stickx = int(b[2])
        sticky = int(b[3])

        # ----------------------------------
        # Override logic (same as your script)
        # ----------------------------------
        button = 0
        stickx = 0
        sticky = 0

        if (150 < i < 160) or (200 < i < 300):
            button = START_BUTTON

        if i > 300:
            sticky = 80

        if (i % 2) == 0:
            button = A_BUTTON

        # ----------------------------------
        # Use the GamePad class
        # ----------------------------------
        pad = GamePad(button, stickx, sticky)

        if i == 1000:
            game2 = generator.copy(game)
        if i >= 1000:
            button2 = 0
            if (i % 8) == 0:
                button2 = A_BUTTON
            pad2 = GamePad(button2, 40, 80)
            game2.step_game(pad2)
            state2 = game2.get_game_state()
            positions_clone_list.append(state2.pos)


        # Step
        game.step_game(pad)

        # Read state
        state = game.get_game_state()
        positions_list.append(state.pos)



    # ----------------------------------
    # Convert to array and animate
    # ----------------------------------
    points = np.array(positions_list)

    positions_clone_list = positions_list[:1000] + positions_clone_list
    points2 = np.array(positions_clone_list)
    # create_3d_rotation_gif(points, points_per_frame=100)
    create_3d_rotation_gif_two_colors(points, points2)
# ---------------------------------------------------------

if __name__ == '__main__':
    run()
