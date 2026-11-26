#!/usr/bin/env python3
"""Bind vanilla 26.3 block implementation classes to generator rules."""

import json
import pathlib
import sys

CLASS_RULES = dict(
    item.split("=", 1)
    for item in """
FireBlock=Fire CactusBlock=Cactus PathBlock=Path MushroomBlock=Mushroom
MultifaceBlock=Multiface VineBlock=Vine HangingMossBlock=HangingMoss
SeaPickleBlock=SeaPickle SnowLayerBlock=SnowLayer BellBlock=Bell
ShelfMushroomBlock=FullBehind GrowingPlantBlock=GrowingPlant
MangrovePropaguleBlock=MangrovePropagule SpeleothemBlock=Speleothem
ScaffoldingBlock=Scaffolding PistonHeadBlock=PistonHead
ChorusPlantBlock=ChorusPlant ChorusFlowerBlock=ChorusFlower
PitcherCropBlock=PitcherCrop BigDripleafBlock=BigDripleaf
BigDripleafStemBlock=BigDripleafStem FarmlandBlock=Farmland CocoaBlock=Cocoa
SugarCaneBlock=SugarCane SporeBlossomBlock=SporeBlossom FrogspawnBlock=Frogspawn
SmallDripleafBlock=SmallDripleaf TallSeagrassBlock=TallSeagrass DoorBlock=Door
BaseRailBlock=RigidBelow LeafLitterBlock=FullBelow WallSignBlock=SolidBehind
GrindstoneBlock=Always RedstoneWallTorchBlock=FullBehind BubbleColumnBlock=BubbleColumn
RedstoneWireBlock=RedstoneWire SoulFireBlock=TagBelow:minecraft:soul_fire_base_blocks
BlockBehaviour=Always CakeBlock=SolidBelow CandleCakeBlock=SolidBelow
BannerBlock=SolidBelow StandingSignBlock=SolidBelow WallBannerBlock=SolidBehind
CarpetBlock=NonAirBelow MossyCarpetBlock=MossCarpet BaseTorchBlock=CenterBelow
CandleBlock=CenterBelow LanternBlock=Lantern HangingRootsBlock=FullAbove
LadderBlock=FullBehind AmethystClusterBlock=FullBehind
TripWireHookBlock=FullBehind BaseCoralWallFanBlock=FullBehind
WallTorchBlock=FullBehind BasePressurePlateBlock=RigidOrCenterBelow
DiodeBlock=RigidBelow BaseCoralPlantTypeBlock=FullBelow
CeilingHangingSignBlock=CenterAbove
FaceAttachedHorizontalDirectionalBlock=FaceAttached
BambooSaplingBlock=SupportsBamboo BambooStalkBlock=SupportsBamboo
""".split()
)
VEGETATION_RULES = dict(
    item.split("=", 1)
    for item in """
LilyPadBlock=LilyPad VegetationBlock=TagBelow:minecraft:supports_vegetation
AzaleaBlock=TagBelow:minecraft:supports_azalea
DryVegetationBlock=TagBelow:minecraft:supports_dry_vegetation
NetherSproutsBlock=TagBelow:minecraft:supports_nether_sprouts
NetherWartBlock=TagBelow:minecraft:supports_nether_wart
WitherRoseBlock=TagBelow:minecraft:supports_wither_rose
CactusFlowerBlock=CactusFlower SeagrassBlock=Seagrass
""".split()
)


def main(directory: str) -> None:
    root = pathlib.Path(directory)
    classes = json.loads((root / "survival-classes.json").read_text())
    placement = json.loads((root / "survival-placement-classes.json").read_text())
    support = json.loads((root / "survival-support-tags.json").read_text())
    rules = {}

    for name, owner in classes.items():
        if owner in CLASS_RULES:
            rules[name] = CLASS_RULES[owner]
    for name, owner in classes.items():
        placed = placement.get(name)
        if owner == "CropBlock" and placed == "CropBlock":
            rules[name] = "Crop"
        if owner == "DoublePlantBlock" and placed == "VegetationBlock":
            rules[name] = "DoublePlant"
        if owner != "VegetationBlock":
            continue
        if placed is None:
            raise ValueError(f"missing placement class for {name}")
        if placed in VEGETATION_RULES:
            rules[name] = VEGETATION_RULES[placed]
        elif placed in {
            "AttachedStemBlock",
            "StemBlock",
            "NetherFungusBlock",
            "NetherRootsBlock",
        }:
            field = "stemSupportBlocks" if placed == "StemBlock" else "supportBlocks"
            tag = support.get(name, {}).get(field)
            if tag is None:
                raise ValueError(f"missing {field} support tag for {name}")
            rules[name] = f"TagBelow:{tag}"

    (root / "survival-rules.json").write_text(
        json.dumps(rules, separators=(",", ":")) + "\n"
    )


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: bind-survival-rules.py <export-directory>")
    main(sys.argv[1])
