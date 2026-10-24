# Style sheet

Every model, every ground colour and every render pass is held to this sheet. It is measured from the target frames the game reproduces: flat-shaded low-poly shapes, a one-pixel outline, a half-resolution pixel grain, one warm sun by day and one teal-blue grade by night. `tools/restyle.py` maps a pack's colours onto the palette below and `just lint` holds every model to the budget.

## What a frame looks like

| | |
|---|---|
| Shading | Every face is one flat value. An object reads as two or three values: lit, side, shaded. No gradients across a face, no specular, no rim light. |
| Outline | A one-pixel dark line on silhouettes and creases, one pixel of the half-resolution image, so two pixels on screen. |
| Resolution | The world is rendered at half the window size and upscaled with nearest filtering: edges step in two-pixel stairs at 1280 wide. The interface is drawn afterwards at full size. |
| Ground | One flat colour per ground material, no texture, no noise. Paint blends colours, never textures. Grass tufts are small three-blade props scattered sparsely, with no footprint. |
| Rocks | Stacked faceted slabs, 100 to 400 triangles, grey by day and blue at night from the grade. A few painted crack lines as darker vertex colour. |
| Trees | Canopies of painted leaf clusters in two greens with a highlight, gnarled trunks with roots, three to four characters tall. |
| Shadows | One warm sun, soft shadows (2048 px, two cascades), softened further by the half-resolution upscale. Soft dark pools under trees and rocks. |
| Night | The same assets under one teal-blue grade: rocks turn blue, greens turn teal. Warm point lights pool on the ground; glowing things keep their own colour with a halo. |
| Polygons | Props in the low hundreds of triangles, a tree a few hundred, a character under four thousand. One material per model. |
| Animation | Few keys, snappy, held poses. A walk is a bob and a stride. Clips play as authored, no blending between actions, at most a 100 ms crossfade. |
| Camera | Perspective, field of view 32°, pitched 55°, three zoom steps, free yaw. No horizon in frame. |
| Effects | Soft halos on glowing things, pools of light on the ground, pops when a pickup appears. Nothing glossy, no bloom: glow is an emissive colour. |
| Sizes | At 1280 wide a character is about 50 px tall, a tree three to four times that, a large rock about a tree. |

## Measurements

Day: sun elevation 52°, warm white, ambient a cool pale green. Night: no sun, dim cool ambient, grade colour `#5680A8` at strength 0.55, warm point lights, ice-blue gem glow.

## Palette

Every vertex colour in a shipped model is one of these. The greens, stone, bark, grass and glow swatches are measured from the frames; the rest sit beside them at the same saturation and value so a character stands in the same light as the trees.

| swatch | hex | use |
|---|---|---|
| grass | #A9D16E | the clearing's ground |
| grass-light | #BADC7B | sunlit patches of ground, tufts |
| tuft | #C9EB7D | the tips of grass tufts |
| dirt | #B8A26A | paths, bare earth |
| stone-ground | #9A9A98 | paved and rocky ground |
| planks | #A67B50 | floors, tables, fences, carts |
| wood-dark | #7A5436 | beams, barrels, the undersides of planks |
| pond | #4F8FA8 | still water |
| leaf-light | #8DC25A | the top of a canopy, highlights |
| leaf | #6AA43E | a canopy's body |
| leaf-dark | #44702A | the underside of a canopy, bushes in shade |
| bark | #6E5C3A | trunks and roots |
| bark-dark | #52453A | the shaded side of a trunk, old stumps |
| stone-light | #AFAEAE | the top of a rock |
| stone | #888989 | a rock's sides |
| stone-dark | #53555C | the shaded side of a rock, walls |
| stone-deep | #45444C | cracks, the base of a rock, iron |
| bone | #E8DFC4 | skeletons, teeth, tusks |
| bone-dark | #B9AE90 | the shaded side of bone, old rope |
| cloth-red | #B04A3C | tunics, banners, tents |
| cloth-blue | #4A6FA5 | tunics, hoods, blankets |
| cloth-green | #5E8C4A | capes, cloaks |
| cloth-purple | #6F4E8A | robes, hats |
| cloth-white | #E6E3D8 | shirts, sails, canvas |
| cloth-black | #2B2A30 | hoods, boots, shadows under hats |
| leather | #7C5A3A | belts, straps, boots, books |
| steel | #9EA6AE | blades, helmets, buckets |
| steel-dark | #5C6670 | chain mail, the shaded side of steel |
| gold | #D9A441 | coins, trim, keys |
| skin | #E0B48C | faces and hands |
| skin-dark | #B07D55 | weathered faces and hands |
| hair | #3B2E25 | dark hair and beards |
| hair-brown | #7A4A32 | auburn hair |
| hair-blond | #C9A25C | fair hair |
| eyes | #141418 | eyes, mouths, the inside of a well |
| glow-ice | #8FD8FF | gems, wisps, magic |
| glow-fire | #FFA43A | flames, lanterns, embers |
| glow-gem | #4DA8FF | the body of a glowing gem |

A vertex colour's alpha says how the surface is lit: 1 is lit by the sun and the lights, 0 glows with its own colour, unlit. Flames, gems and eyes that shine are the only use.

## Conventions

- One unit is one metre. A character stands about 1.7 m tall; a door is 2 m; a tree 5 to 7 m.
- A prop's origin is the middle of its base, the base 2 cm below the origin so it never fights the terrain. A character's origin is between its feet on the ground.
- A model faces +Z at yaw 0. The engine turns it by its placement's yaw or the actor's facing.
- Flat normals: every triangle has its own, no vertex is shared between faces that aren't coplanar. Vertex colours carry every colour; a model has no texture, no tangents and no texture coordinates.
- A character's rig has at most 64 joints and one skinned mesh; hats, hoods and capes are skinned to the joint they ride on. Clips are named by the action they play: `idle`, `walk`, `run`, `attack`, `death`, `talk`, and the pose `sit`.

## Budget

| | limit |
|---|---|
| prop model | 2 MB, 2 000 triangles, 1 material |
| character model | 4 MB, 4 000 triangles, 64 joints, 1 material |
| clip | 100 ms to 60 s |
| point lights | 8 per area |
| props placed | 2 000 per area, scatter included |
