// Game constants used for display.

export const BUTTONS = [
  { name: 'A', mask: 0x8000 },
  { name: 'B', mask: 0x4000 },
  { name: 'Z', mask: 0x2000 },
  { name: 'S', mask: 0x1000, title: 'Start' },
  { name: 'L', mask: 0x0020 },
  { name: 'R', mask: 0x0010 },
  { name: 'C^', mask: 0x0008 },
  { name: 'Cv', mask: 0x0004 },
  { name: 'C<', mask: 0x0002 },
  { name: 'C>', mask: 0x0001 },
  { name: 'D^', mask: 0x0800 },
  { name: 'Dv', mask: 0x0400 },
  { name: 'D<', mask: 0x0200 },
  { name: 'D>', mask: 0x0100 },
];

// gCurrCourseNum
const COURSES = [
  'Castle / hub',
  'Bob-omb Battlefield',
  "Whomp's Fortress",
  'Jolly Roger Bay',
  'Cool, Cool Mountain',
  "Big Boo's Haunt",
  'Hazy Maze Cave',
  'Lethal Lava Land',
  'Shifting Sand Land',
  'Dire, Dire Docks',
  "Snowman's Land",
  'Wet-Dry World',
  'Tall, Tall Mountain',
  'Tiny-Huge Island',
  'Tick Tock Clock',
  'Rainbow Ride',
  'Bowser in the Dark World',
  'Bowser in the Fire Sea',
  'Bowser in the Sky',
  "Princess's Secret Slide",
  'Cavern of the Metal Cap',
  'Tower of the Wing Cap',
  'Vanish Cap under the Moat',
  'Wing Mario over the Rainbow',
  "Secret Aquarium",
  'Ending',
];

export function courseName(n) {
  return COURSES[n] ?? `Course ${n}`;
}
