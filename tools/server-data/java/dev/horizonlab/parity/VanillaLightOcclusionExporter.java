package dev.horizonlab.parity;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.Gson;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;

/** Exports vanilla's per-state 16x16 face-occlusion masks for the server light engine. */
public final class VanillaLightOcclusionExporter {
    private static final int FACE_CELLS = 16;

    private static String mask(VoxelShape shape, Direction direction) {
        boolean[] cells = new boolean[FACE_CELLS * FACE_CELLS];
        for (AABB box : shape.toAabbs()) {
            for (int v = 0; v < FACE_CELLS; v++) {
                double vc = (v + 0.5) / FACE_CELLS;
                for (int u = 0; u < FACE_CELLS; u++) {
                    double uc = (u + 0.5) / FACE_CELLS;
                    double minU, maxU, minV, maxV;
                    switch (direction.getAxis()) {
                        case X -> { minU = box.minZ; maxU = box.maxZ; minV = box.minY; maxV = box.maxY; }
                        case Y -> { minU = box.minX; maxU = box.maxX; minV = box.minZ; maxV = box.maxZ; }
                        case Z -> { minU = box.minX; maxU = box.maxX; minV = box.minY; maxV = box.maxY; }
                        default -> throw new AssertionError(direction.getAxis());
                    }
                    if (uc >= minU && uc < maxU && vc >= minV && vc < maxV) {
                        cells[v * FACE_CELLS + u] = true;
                    }
                }
            }
        }
        StringBuilder out = new StringBuilder(64);
        for (int nibble = 0; nibble < 64; nibble++) {
            int value = 0;
            for (int bit = 0; bit < 4; bit++) {
                if (cells[nibble * 4 + bit]) value |= 1 << bit;
            }
            out.append(Character.forDigit(value, 16));
        }
        return out.toString();
    }

    private static boolean fullUnion(String a, String b) {
        for (int i = 0; i < a.length(); i++) {
            int x = Character.digit(a.charAt(i), 16);
            int y = Character.digit(b.charAt(i), 16);
            if ((x | y) != 15) return false;
        }
        return true;
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        List<Map<String, Integer>> idsByAxis = new ArrayList<>();
        List<List<String>> masksByAxis = new ArrayList<>();
        List<List<VoxelShape>> shapesByAxis = new ArrayList<>();
        for (int axis = 0; axis < 3; axis++) {
            idsByAxis.add(new LinkedHashMap<>());
            masksByAxis.add(new ArrayList<>());
            shapesByAxis.add(new ArrayList<>());
        }

        JsonArray states = new JsonArray();
        for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
            JsonArray faces = new JsonArray();
            for (Direction direction : Direction.values()) {
                int axis = direction.getAxis().ordinal();
                VoxelShape shape = state.getFaceOcclusionShape(direction);
                String mask = mask(shape, direction);
                Map<String, Integer> ids = idsByAxis.get(axis);
                Integer id = ids.get(mask);
                if (id == null) {
                    id = ids.size();
                    ids.put(mask, id);
                    masksByAxis.get(axis).add(mask);
                    shapesByAxis.get(axis).add(shape);
                }
                faces.add(id);
            }
            states.add(faces);
        }

        // Validate the compact 16x16 masks against vanilla's shape merger for every
        // unique face-shape pair in each axis before writing a bundle for Rust.
        int comparisons = 0;
        for (int axis = 0; axis < 3; axis++) {
            List<String> masks = masksByAxis.get(axis);
            List<VoxelShape> shapes = shapesByAxis.get(axis);
            for (int a = 0; a < masks.size(); a++) {
                for (int b = 0; b < masks.size(); b++) {
                    boolean vanilla = Shapes.faceShapeOccludes(shapes.get(a), shapes.get(b));
                    boolean compact = fullUnion(masks.get(a), masks.get(b));
                    if (vanilla != compact) {
                        throw new AssertionError("16x16 light mask disagrees with vanilla at axis " + axis + ", shapes " + a + "," + b);
                    }
                    comparisons++;
                }
            }
        }

        JsonObject result = new JsonObject();
        result.addProperty("format", 1);
        JsonArray axes = new JsonArray();
        for (List<String> masks : masksByAxis) {
            JsonArray entries = new JsonArray();
            masks.forEach(entries::add);
            axes.add(entries);
        }
        result.add("faces_by_axis", axes);
        result.add("states", states);
        result.addProperty("vanilla_pair_checks", comparisons);
        Files.writeString(Path.of(args[0]), new Gson().toJson(result), StandardCharsets.UTF_8);
        System.out.println("Exported light occlusion for " + states.size() + " block states; validated " + comparisons + " shape pairs");
    }
}
