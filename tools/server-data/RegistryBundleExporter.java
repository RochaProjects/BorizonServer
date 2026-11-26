import java.nio.file.Files;
import java.nio.file.Path;
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import net.minecraft.SharedConstants;
import net.minecraft.commands.Commands;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistrySynchronization;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.NbtIo;
import net.minecraft.nbt.NbtOps;
import net.minecraft.resources.ResourceKey;
import net.minecraft.tags.TagNetworkSerialization;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.permissions.PermissionSet;
import net.minecraft.world.level.WorldDataConfiguration;

/** Emits the exact dynamic registry payloads used by the 26.3 server's config packets. */
public final class RegistryBundleExporter {
  private static final int EXPECTED_REGISTRIES = 32;

  private RegistryBundleExporter() {}

  public static void main(String[] args) {
    if (args.length != 1) {
      throw new IllegalArgumentException("usage: RegistryBundleExporter <resource-output-dir>");
    }

    try {
      export(Path.of(args[0]));
    } catch (Exception exception) {
      throw new RuntimeException("could not export Minecraft 26.3 synchronized registries", exception);
    }
  }

  private static void export(Path output) throws Exception {
    Files.createDirectories(output);
    SharedConstants.tryDetectVersion();
    Bootstrap.bootStrap();

    PackRepository repository = ServerPacksSource.createVanillaTrustedRepository();
    repository.reload();
    if (!repository.getAvailableIds().contains("vanilla")) {
      throw new IllegalStateException("server jar did not expose the vanilla data pack");
    }
    repository.setSelected(List.of("vanilla"));

    var packConfig = new WorldLoader.PackConfig(repository, WorldDataConfiguration.DEFAULT, false, true);
    var init = new WorldLoader.InitConfig(
        packConfig, Commands.CommandSelection.DEDICATED, PermissionSet.ALL_PERMISSIONS);
    WorldLoader.load(
            init,
            context -> new WorldLoader.DataLoadOutput<>(null, context.datapackDimensions()),
            (manager, reloadable, registries, cookie) -> {
              var registryPayloads = new LinkedHashMap<
                  ResourceKey<? extends Registry<?>>,
                  List<RegistrySynchronization.PackedRegistryEntry>>();
              RegistrySynchronization.packRegistries(
                  registries.compositeAccess().createSerializationContext(NbtOps.INSTANCE),
                  registries.getAccessFrom(RegistryLayer.WORLD),
                  Set.of(),
                  registryPayloads::put);
              if (registryPayloads.size() != EXPECTED_REGISTRIES) {
                throw new IllegalStateException(
                    "expected " + EXPECTED_REGISTRIES + " synchronized registries, found "
                        + registryPayloads.size());
              }

              int entryCount = 0;
              JsonObject registryOrder = new JsonObject();
              for (Map.Entry<
                      ResourceKey<? extends Registry<?>>,
                      List<RegistrySynchronization.PackedRegistryEntry>> registry :
                  registryPayloads.entrySet()) {
                CompoundTag entries = new CompoundTag();
                JsonArray orderedIds = new JsonArray();
                for (var entry : registry.getValue()) {
                  CompoundTag encoded = new CompoundTag();
                  encoded.putBoolean("_borizon_data_present", entry.data().isPresent());
                  if (entry.data().isPresent()) {
                    encoded.put("data", entry.data().get());
                  }
                  entries.put(entry.id().toString(), encoded);
                  orderedIds.add(entry.id().toString());
                }
                if (entries.isEmpty()) {
                  throw new IllegalStateException("empty synchronized registry " + registry.getKey());
                }
                String id = registry.getKey().identifier().toString();
                registryOrder.add(id, orderedIds);
                int colon = id.indexOf(':');
                Path file = output.resolve("registry-sync")
                    .resolve(id.substring(0, colon))
                    .resolve(id.substring(colon + 1) + ".nbt");
                try {
                  Files.createDirectories(file.getParent());
                  NbtIo.write(entries, file);
                } catch (java.io.IOException exception) {
                  throw new RuntimeException("could not write " + file, exception);
                }
                entryCount += entries.size();
              }
              try {
                Files.writeString(output.resolve("registry-order.json"), new Gson().toJson(registryOrder));
              } catch (java.io.IOException exception) {
                throw new RuntimeException("could not write registry-order.json", exception);
              }
              CompoundTag encodedTags = new CompoundTag();
              int tagCount = 0;
              for (var registryTags : TagNetworkSerialization.serializeTagsToNetwork(registries).entrySet()) {
                CompoundTag tags = new CompoundTag();
                registryTags.getValue().tags().forEach((name, ids) -> {
                  tags.putIntArray(name.toString(), ids.toIntArray());
                });
                encodedTags.put(registryTags.getKey().identifier().toString(), tags);
                tagCount += tags.size();
              }
              Path tagsFile = output.resolve("registry-tags.nbt");
              try {
                NbtIo.write(encodedTags, tagsFile);
              } catch (java.io.IOException exception) {
                throw new RuntimeException("could not write " + tagsFile, exception);
              }
              System.out.println(
                  "Exported " + registryPayloads.size() + " vanilla 26.3 registries ("
                      + entryCount + " entries) and " + tagCount + " tags to "
                      + output.resolve("registry-sync"));
              return null;
            },
            Runnable::run,
            Runnable::run)
        .join();
  }
}
