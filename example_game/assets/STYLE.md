# Style sheet

Every model, every ground colour and every render pass is held to this sheet. It is drawn from the concept board the game reproduces: chunky low-poly models in saturated colours under warm light, deep shade beside faces lit nearly white, blob trees and lush grass, clear bright water, seen from above. Every pack model was restyled onto the palette below, and `just lint` holds every model to the budget.

## What a frame looks like

| | |
|---|---|
| Shading | A face takes the sun, the sky's light and the lamps near it, rough and matte but for metal, which catches a soft sheen, and creases and feet darken where things stand close (ambient occlusion). The light is mapped to the screen by a filmic curve that deepens the shade and lifts the light, the same for every area. Faceted things (rocks, walls, crates) keep flat faces; rounded things (canopies, bushes, bodies, barrels) are shaded smooth across their faces. No outline. |
| Light | One warm sun with soft-edged shadows that blur as they fall further from what casts them, the sky's light filling the shade, so shadows stay deep and coloured rather than black, and the shadows of clouds drifting over the ground on the wind. Shafts of sunlight show in the air where it breaks through a canopy. What glows (lamps, gems, embers) blooms softly over its rim and pools light on the ground. |
| Air | A real sky: the sun's light scattered through an atmosphere, blue overhead and pale at the horizon, reddening as the sun sets, and the far distance sinking into that blue haze. |
| Ground | A smooth coloured surface, painted terrains blending into one another, under a carpet of grass blades and flowers, a shade deeper than the swatches of the same name so the wide sunlit ground stays under what stands on it. |
| Indoors | No sun and no sky: black beyond the walls, a dim violet ambient, and warm lamps pooling light, a hearth lighting the room and torches the walls beside them. |
| Rocks | Faceted low-poly slabs in cool grey-mauve, lit faces paler, 100 to 400 triangles. |
| Trees | Rounded canopies of lumpy leaf clusters shaded smooth, a lit yellow-green on top deepening to a dark green beneath, on bent trunks with roots; three to four characters tall. Bushes the same, smaller. |
| Water | Bright cyan, clear in the shallows and deep blue further out, its surface a drifting net of light cells, with bands and dashes of white foam along the shore and rings spreading now and then. |
| Camera | Perspective, field of view 32°, pitched 55°, three zoom steps, free yaw. |
| Polygons | Props in the low hundreds of triangles, a tree under two thousand, a character under four thousand. One material per model. |
| Animation | Few keys, snappy, held poses. A walk is a bob and a stride. Clips play as authored, no blending between actions, at most a 100 ms crossfade. |
| Sizes | At 1280 wide a character is about 50 px tall, a tree three to four times that, a large rock about a tree. |

## Measurements

Day: sun elevation 52°, warm white, ambient a cool blue under the sky's light, no grade. Indoors: no sun, a dark sky, ambient `#64547C`, no grade, a hearth `#FFC48C` and torches `#FFCE9C`, ice-blue gem glow.

## Palette

Every vertex colour in a shipped model is one of these. The greens, earth, stone, wood and water swatches are taken from the board; the rest sit beside them at the same saturation and value so a character stands in the same light as the trees.

| swatch | hex | use |
|---|---|---|
| grass | #7CBB47 | the clearing's ground |
| grass-light | #A3D455 | sunlit patches of ground, tufts |
| tuft | #CDE86C | the tips of grass tufts |
| dirt | #C8955E | paths, bare earth |
| stone-ground | #A89EA9 | paved and rocky ground |
| planks | #D9915A | floors, tables, fences, carts |
| wood-dark | #A05A47 | beams, barrels, the undersides of planks |
| pond | #22A6D2 | still water |
| leaf-light | #C4E64F | the top of a canopy, highlights |
| leaf | #6DBB3F | a canopy's body |
| leaf-dark | #3A7D33 | the underside of a canopy, bushes in shade |
| bark | #7A5236 | trunks and roots |
| bark-dark | #553A2B | the shaded side of a trunk, old stumps |
| stone-light | #CFC6D2 | the top of a rock |
| stone | #9E95A6 | a rock's sides |
| stone-dark | #6A6478 | the shaded side of a rock, walls |
| stone-deep | #4A4556 | cracks, the base of a rock, iron |
| bone | #EADFC6 | skeletons, teeth, tusks |
| bone-dark | #BDAF92 | the shaded side of bone, old rope |
| cloth-red | #D45A4A | tunics, banners, tents |
| cloth-blue | #3F7FD6 | tunics, hoods, blankets |
| cloth-green | #5FA04E | capes, cloaks |
| cloth-purple | #8A5FB8 | robes, hats |
| cloth-white | #F0ECE4 | shirts, sails, canvas |
| cloth-black | #343040 | hoods, boots, shadows under hats |
| leather | #8C5C3C | belts, straps, boots, books |
| steel | #B4BCC8 | blades, helmets, buckets |
| steel-dark | #6A7384 | chain mail, the shaded side of steel |
| gold | #F2B544 | coins, trim, keys |
| skin | #F0C09A | faces and hands |
| skin-dark | #C58D62 | weathered faces and hands |
| hair | #3E2E28 | dark hair and beards |
| hair-brown | #8A4E34 | auburn hair |
| hair-blond | #D9B25F | fair hair |
| eyes | #1A1A22 | eyes, mouths, the inside of a well |
| glow-ice | #8FE8FF | gems, wisps, magic |
| glow-fire | #FFB347 | flames, lanterns, embers |
| glow-gem | #4DB8FF | the body of a glowing gem |

A vertex colour's alpha says how the surface is lit: 1 is lit by the sun and the lights, 0 glows with its own colour, unlit. Flames, gems and eyes that shine are the only use.

## Conventions

- One unit is one metre. A character stands about 1.7 m tall; a door is 2 m; a tree 5 to 7 m.
- A prop's origin is the middle of its base, the base 2 cm below the origin so it never fights the terrain. A character's origin is between its feet on the ground.
- A model faces +Z at yaw 0. The engine turns it by its placement's yaw or the actor's facing.
- Normals: an edge between two faces that turn less than 48° from each other shades smooth across it, so a rounded surface (a body, a barrel, a trunk) reads round, and an edge that turns more stays a crisp corner (a crate, a plank, a rock's facets); the kit may set a model's own angle (`smooth_angle`) or mark it `smooth` to round every edge. Vertex colours carry every colour; a model has no texture, no tangents and no texture coordinates.
- A character's rig has at most 64 joints and one skinned mesh; hats, hoods and capes are skinned to the joint they ride on. Clips are named by the action they play: `idle`, `walk`, `run`, `attack`, `death`, `talk`, and the pose `sit`.

## Budget

| | limit |
|---|---|
| prop model | 2 MB, 2 000 triangles, 1 material |
| character model | 4 MB, 4 000 triangles, 64 joints, 1 material |
| clip | 100 ms to 60 s |
| point lights | 8 per area |
| props placed | 2 000 per area, scatter included |
