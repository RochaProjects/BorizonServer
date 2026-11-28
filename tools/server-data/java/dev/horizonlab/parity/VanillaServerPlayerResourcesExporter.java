package dev.horizonlab.parity;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.syncher.EntityDataAccessor;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.ai.attributes.Attribute;
import net.minecraft.world.entity.ai.attributes.AttributeInstance;
import net.minecraft.world.entity.ai.attributes.AttributeSupplier;
import net.minecraft.world.entity.ai.attributes.DefaultAttributes;
import net.minecraft.world.entity.ai.attributes.RangedAttribute;
import net.minecraft.world.entity.player.Player;

/** Export player float IDs and entity attribute defaults using server classes. */
public final class VanillaServerPlayerResourcesExporter {
    private static Field field(Class<?> type, String name) throws Exception {
        Field field = type.getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }

    private static String bits(double value) {
        return String.format("%016x", Double.doubleToLongBits(value));
    }

    private static JsonObject attributes() throws Exception {
        List<Holder<Attribute>> attributes = new ArrayList<>();
        for (Attribute attribute : BuiltInRegistries.ATTRIBUTE) {
            attributes.add(BuiltInRegistries.ATTRIBUTE.wrapAsHolder(attribute));
        }

        JsonObject root = new JsonObject();
        root.addProperty(
                "data_version",
                net.minecraft.SharedConstants.getCurrentVersion().dataVersion().version());
        JsonArray definitions = new JsonArray();
        for (Holder<Attribute> holder : attributes) {
            Attribute attribute = holder.value();
            JsonObject definition = new JsonObject();
            definition.addProperty("id", BuiltInRegistries.ATTRIBUTE.getKey(attribute).toString());
            definition.addProperty("default", bits(attribute.getDefaultValue()));
            definition.addProperty("syncable", attribute.isClientSyncable());
            if (attribute instanceof RangedAttribute ranged) {
                definition.addProperty("min", bits(ranged.getMinValue()));
                definition.addProperty("max", bits(ranged.getMaxValue()));
            }
            definitions.add(definition);
        }
        root.add("attributes", definitions);

        JsonObject entities = new JsonObject();
        for (EntityType<?> type : BuiltInRegistries.ENTITY_TYPE) {
            if (!DefaultAttributes.hasSupplier(type)) continue;
            AttributeSupplier supplier = supplier(type);
            JsonObject bases = new JsonObject();
            for (Holder<Attribute> holder : attributes) {
                if (!supplier.hasAttribute(holder)) continue;
                AttributeInstance instance = supplier.createInstance(value -> {}, holder);
                if (!instance.getModifiers().isEmpty()) {
                    throw new IllegalStateException("default attribute modifiers need export support");
                }
                bases.addProperty(
                        BuiltInRegistries.ATTRIBUTE.getKey(holder.value()).toString(),
                        bits(supplier.getBaseValue(holder)));
            }
            Identifier id = BuiltInRegistries.ENTITY_TYPE.getKey(type);
            entities.add(id.toString(), bases);
        }
        root.add("entities", entities);
        return root;
    }

    @SuppressWarnings("unchecked")
    private static AttributeSupplier supplier(EntityType<?> type) {
        return DefaultAttributes.getSupplier((EntityType<? extends LivingEntity>) type);
    }

    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        Path output = Path.of(args[0]);
        Files.createDirectories(output);
        Files.writeString(output.resolve("attributes.json"), attributes().toString());

        EntityDataAccessor<Float> health =
                (EntityDataAccessor<Float>) field(LivingEntity.class, "DATA_HEALTH_ID").get(null);
        EntityDataAccessor<Float> absorption =
                (EntityDataAccessor<Float>) field(Player.class, "DATA_PLAYER_ABSORPTION_ID").get(null);
        JsonObject ids = new JsonObject();
        ids.addProperty("health", health.id());
        ids.addProperty("absorption", absorption.id());
        Files.writeString(output.resolve("player-float-ids.json"), ids.toString());
    }
}
