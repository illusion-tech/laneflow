// 最小跨语言 LFRE writer（C++）。构造固定最小模块并写出 size-prefixed bytes。
// 用途：证明非 Rust writer 产出的 bytes 能被生产 reader 接受（golden fixture）。
// 再生成步骤与钉版来源见 tools/lfre-crosslang-writer/README.md。
#include <cstdint>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <vector>

#include "flatbuffers/flatbuffers.h"
#include "road-editing_generated.h"

namespace v1 = LaneFlow::RoadEditing::V1;

namespace {

// 与 Rust RoadEditingProvenance::direct 的冻结值一致
// （crates/laneflow-compiler/src/road_editing/model.rs）。
const uint8_t kInputsDigest[32] = {
    0x6b, 0x27, 0xd0, 0xf7, 0x66, 0x93, 0xbc, 0xd3, 0x86, 0xac, 0x13, 0xdf, 0x72, 0x4e, 0x30, 0xf5,
    0xfb, 0x5a, 0xd3, 0xb9, 0xa1, 0x52, 0xa5, 0xe1, 0xf8, 0x8d, 0xe1, 0xa6, 0x24, 0xce, 0xa8, 0xaa,
};
const uint8_t kFrontendOptionsDigest[32] = {
    0xb1, 0x62, 0x1e, 0x4a, 0x2d, 0xb8, 0xd7, 0x17, 0xb6, 0x50, 0x6b, 0x0a, 0xfb, 0x6f, 0xef, 0x5b,
    0xd4, 0xd5, 0x15, 0x6e, 0xcf, 0xe8, 0x87, 0xc5, 0xab, 0xf3, 0x6d, 0x08, 0x86, 0x9c, 0x78, 0x92,
};

template <typename T>
::flatbuffers::Offset<::flatbuffers::Vector<::flatbuffers::Offset<T>>> EmptyVectorOf(
    ::flatbuffers::FlatBufferBuilder &fbb) {
  return fbb.CreateVector(std::vector<::flatbuffers::Offset<T>>{});
}

::flatbuffers::Offset<::flatbuffers::Vector<::flatbuffers::Offset<::flatbuffers::String>>>
EmptyStringVector(::flatbuffers::FlatBufferBuilder &fbb) {
  return fbb.CreateVector(std::vector<::flatbuffers::Offset<::flatbuffers::String>>{});
}

}  // namespace

int main(int argc, char **argv) {
  if (argc != 2) {
    std::cerr << "usage: writer <output.lfre>\n";
    return EXIT_FAILURE;
  }

  ::flatbuffers::FlatBufferBuilder fbb(256);

  const v1::Digest256 inputs_digest{::flatbuffers::span<const uint8_t, 32>(kInputsDigest)};
  const v1::Digest256 options_digest{
      ::flatbuffers::span<const uint8_t, 32>(kFrontendOptionsDigest)};
  const auto provenance = v1::CreateProvenanceDirect(
      fbb, v1::ProvenanceKind_Direct, "laneflow-road-editing-direct-v1", &inputs_digest,
      &options_digest, nullptr, "cross-language writer fixture");

  const std::vector<::flatbuffers::Offset<::flatbuffers::String>> no_imports;
  const auto header = v1::CreateModuleHeaderDirect(
      fbb, "city", "roads/crosslang-writer", &no_imports, provenance);

  const auto frame = v1::CreateCanonicalFrameDirect(fbb, "frame");

  const v1::Vec3F64 curve_start(0.0, 0.0, 0.0);
  const v1::Vec3F64 curve_end(10.0, 0.0, 0.0);
  const auto line = v1::CreateLineSegment(fbb, &curve_end);
  const auto segment = v1::CreateCurveSegment(
      fbb, v1::CurveSegmentGeometry_LineSegment, line.Union());
  const auto segments =
      fbb.CreateVector(std::vector<::flatbuffers::Offset<v1::CurveSegment>>{segment});
  const auto geometry = v1::CreateCurveProgram(fbb, &curve_start, segments);
  const auto edge = v1::CreateLaneEdge(
      fbb, fbb.CreateString("edge-a"), 10.0, EmptyStringVector(fbb), geometry);

  const auto root = v1::CreateRoadEditingSource(
      fbb,
      /*format_version=*/4,
      header,
      v1::GeometryAccuracyProfile_Balanced5Cm,
      v1::GeometryDirectionProfile_Balanced2Deg,
      EmptyVectorOf<v1::RoadAlignment>(fbb),
      EmptyVectorOf<v1::RoadCorridor>(fbb),
      EmptyVectorOf<v1::RoadSection>(fbb),
      EmptyVectorOf<v1::AuthoringLane>(fbb),
      fbb.CreateVector(std::vector<::flatbuffers::Offset<v1::LaneEdge>>{edge}),
      EmptyVectorOf<v1::Junction>(fbb),
      EmptyVectorOf<v1::Movement>(fbb),
      EmptyVectorOf<v1::ManeuverPath>(fbb),
      EmptyVectorOf<v1::ManeuverGate>(fbb),
      EmptyVectorOf<v1::WaitingZone>(fbb),
      EmptyVectorOf<v1::StopLine>(fbb),
      EmptyVectorOf<v1::SignalGroup>(fbb),
      EmptyVectorOf<v1::SignalController>(fbb),
      EmptyVectorOf<v1::SignalPhase>(fbb),
      EmptyVectorOf<v1::ParkingFacility>(fbb),
      EmptyVectorOf<v1::ParkingSpace>(fbb),
      EmptyVectorOf<v1::LaneGroup>(fbb),
      EmptyVectorOf<v1::FacilityBand>(fbb),
      EmptyVectorOf<v1::ParticipantClass>(fbb),
      EmptyVectorOf<v1::AccessRule>(fbb),
      EmptyVectorOf<v1::VehicleProfile>(fbb),
      fbb.CreateVector(std::vector<::flatbuffers::Offset<v1::CanonicalFrame>>{frame}),
      EmptyVectorOf<v1::ConflictZone>(fbb),
      EmptyVectorOf<v1::ParticipantStream>(fbb),
      EmptyVectorOf<v1::ConflictZoneRegion>(fbb),
      EmptyVectorOf<v1::RightOfWayPolicySet>(fbb));

  v1::FinishSizePrefixedRoadEditingSourceBuffer(fbb, root);

  std::ofstream output(argv[1], std::ios::binary | std::ios::trunc);
  output.write(reinterpret_cast<const char *>(fbb.GetBufferPointer()),
               static_cast<std::streamsize>(fbb.GetSize()));
  output.close();
  if (!output) {
    std::cerr << "failed to write " << argv[1] << "\n";
    return EXIT_FAILURE;
  }
  std::cout << "wrote " << fbb.GetSize() << " bytes to " << argv[1] << "\n";
  return EXIT_SUCCESS;
}
