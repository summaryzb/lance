/*
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
package org.lance.index;

import org.lance.Dataset;
import org.lance.Fragment;
import org.lance.FragmentMetadata;
import org.lance.FragmentOperation;
import org.lance.WriteParams;
import org.lance.index.scalar.ScalarIndexParams;
import org.lance.index.scalar.ZoneStats;

import org.apache.arrow.memory.BufferAllocator;
import org.apache.arrow.memory.RootAllocator;
import org.apache.arrow.vector.IntVector;
import org.apache.arrow.vector.VectorSchemaRoot;
import org.apache.arrow.vector.types.pojo.ArrowType;
import org.apache.arrow.vector.types.pojo.Field;
import org.apache.arrow.vector.types.pojo.Schema;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.nio.file.Path;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.Optional;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;

/** Verifies getZonemapStats returns identical ZoneStats on warm vs cold path. */
public class ZonemapStatsCacheJniTest {

  // Mirror ZonemapStatsTest.intSchema() / writeIntFragment(...) — keep these
  // duplicated rather than extract a shared helper, to align with the existing
  // test convention (no project-wide LanceTestUtil exists).
  private static Schema intSchema() {
    return new Schema(
        Arrays.asList(
            Field.nullable("id", new ArrowType.Int(32, true)),
            Field.nullable("value", new ArrowType.Int(32, true))),
        null);
  }

  private Dataset writeIntFragment(
      BufferAllocator allocator, String path, long version, int startValue, int rowCount) {
    Schema schema = intSchema();
    List<FragmentMetadata> metas;
    try (VectorSchemaRoot root = VectorSchemaRoot.create(schema, allocator)) {
      root.allocateNew();
      IntVector idVec = (IntVector) root.getVector("id");
      IntVector valVec = (IntVector) root.getVector("value");
      for (int i = 0; i < rowCount; i++) {
        idVec.setSafe(i, startValue + i);
        valVec.setSafe(i, (startValue + i) * 10);
      }
      root.setRowCount(rowCount);
      metas = Fragment.create(path, allocator, root, new WriteParams.Builder().build());
    }
    FragmentOperation.Append appendOp = new FragmentOperation.Append(metas);
    return Dataset.commit(allocator, path, appendOp, Optional.of(version));
  }

  @Test
  public void warm_call_returns_same_data_as_cold(@TempDir Path tempDir) throws Exception {
    String path = tempDir.resolve("warm_eq").toString();
    try (BufferAllocator allocator = new RootAllocator()) {
      try (Dataset ds =
          Dataset.create(allocator, path, intSchema(), new WriteParams.Builder().build())) {
        // empty
      }
      // Two fragments to make sure the multi-segment cache path is exercised.
      writeIntFragment(allocator, path, 1, 0, 50).close();
      writeIntFragment(allocator, path, 2, 50, 50).close();

      try (Dataset dataset = Dataset.open(path, allocator)) {
        ScalarIndexParams params = ScalarIndexParams.create("zonemap", "{}");
        IndexParams indexParams = IndexParams.builder().setScalarIndexParams(params).build();
        dataset.createIndex(
            Collections.singletonList("value"),
            IndexType.ZONEMAP,
            Optional.of("value_zm"),
            indexParams,
            true);

        List<ZoneStats> cold = dataset.getZonemapStats("value");
        List<ZoneStats> warm = dataset.getZonemapStats("value");
        assertNotNull(cold);
        assertNotNull(warm);
        assertFalse(cold.isEmpty(), "expected non-empty zonemap stats");
        assertEquals(cold.size(), warm.size());
        for (int i = 0; i < cold.size(); i++) {
          assertEquals(cold.get(i).getFragmentId(), warm.get(i).getFragmentId());
          assertEquals(cold.get(i).getZoneStart(), warm.get(i).getZoneStart());
          assertEquals(cold.get(i).getZoneLength(), warm.get(i).getZoneLength());
          assertEquals(cold.get(i).getNullCount(), warm.get(i).getNullCount());
          assertEquals(cold.get(i).getMin(), warm.get(i).getMin());
          assertEquals(cold.get(i).getMax(), warm.get(i).getMax());
        }
      }
    }
  }
}
