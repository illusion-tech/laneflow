// 最小跨语言 LFRE writer（C++）。构造固定最小模块并写出 size-prefixed bytes。
// 用途：证明非 Rust writer 产出的 bytes 能被生产 reader 接受（golden fixture）。
// 再生成步骤与钉版来源见 tools/lfre-crosslang-writer/README.md。
#include <algorithm>
#include <cstdint>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <string>
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


}  // namespace

int main(int argc, char **argv) {
  const bool reverse = argc == 3 && std::string(argv[2]) == "--reverse";
  if (argc != 2 && !reverse) {
    std::cerr << "usage: writer <output.lfre> [--reverse]\n";
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
  constexpr double grade = 40.0 / 399.0;
  const v1::Vec3F64 curve_end(0.0, 40.0 * grade, 40.0);
  const auto line = v1::CreateLineSegment(fbb, &curve_end);
  const auto segment = v1::CreateCurveSegment(
      fbb, v1::CurveSegmentGeometry_LineSegment, line.Union());
  const auto spiral = v1::CreateClothoidSegment(fbb, 16.0, 1.0 / 35.0, 0.0, grade);
  const auto transition = v1::CreateCurveSegment(
      fbb, v1::CurveSegmentGeometry_ClothoidSegment, spiral.Union(), 0, true);
  const auto arc = v1::CreateCircularArcSegment(fbb, 45.0, 35.0, grade);
  const auto bend = v1::CreateCurveSegment(
      fbb, v1::CurveSegmentGeometry_CircularArcSegment, arc.Union());
  const auto segments =
      fbb.CreateVector(std::vector<::flatbuffers::Offset<v1::CurveSegment>>{segment, bend, transition});
  const auto geometry = v1::CreateCurveProgram(fbb, &curve_start, segments);
  const auto alignment = v1::CreateRoadAlignmentDirect(fbb, "alignment", "frame", geometry);
  const char *section_ref = "road>section";
  const char *lane_ref = "road>section>A";
  const auto element = v1::CreateCorridorElementDirect(fbb, v1::CorridorElementKind_RoadSection, section_ref);
  std::vector<::flatbuffers::Offset<v1::CorridorElement>> elements{element};
  if (reverse) {
    elements.push_back(v1::CreateCorridorElementDirect(fbb, v1::CorridorElementKind_RoadSection, "road>opposite"));
  }
  const auto corridor = v1::CreateRoadCorridorDirect(fbb, "road", "alignment", 0.0,
      v1::StationEndKind_AlignmentEnd, 0.0, section_ref, lane_ref, &elements);
  std::vector<::flatbuffers::Offset<v1::LaneEdge>> edges;
  std::vector<::flatbuffers::Offset<v1::AuthoringLane>> lanes;
  std::vector<::flatbuffers::Offset<::flatbuffers::String>> lane_refs;
  std::vector<::flatbuffers::Offset<::flatbuffers::String>> opposite_refs;
  const v1::LinearWidthProfile width(3.5, 3.5);
  auto add_lane = [&](const std::string &key, std::vector<double> cuts, bool backward) {
    if (backward) std::reverse(cuts.begin(), cuts.end());
    for (double &station : cuts) station = station * 401.0 / 399.0;
    std::vector<::flatbuffers::Offset<::flatbuffers::String>> chain;
    for (size_t i = 0; i <= cuts.size(); ++i) {
      const size_t edge_index = backward ? cuts.size() - i : i;
      const std::string edge = key + "-" + std::to_string(edge_index);
      chain.push_back(fbb.CreateString(edge));
      std::vector<::flatbuffers::Offset<::flatbuffers::String>> successors;
      if (i < cuts.size()) successors.push_back(fbb.CreateString(key + "-" + std::to_string(backward ? edge_index - 1 : edge_index + 1)));
      edges.push_back(v1::CreateLaneEdgeDirect(fbb, edge.c_str(), 10.0, &successors));
    }
    const char *owner = backward ? "road>opposite" : section_ref;
    (backward ? opposite_refs : lane_refs).push_back(fbb.CreateString(std::string(owner) + ">" + key));
    lanes.push_back(v1::CreateAuthoringLaneDirect(fbb, key.c_str(), &chain,
        backward ? v1::LaneDirection_Backward : v1::LaneDirection_Forward, &width, nullptr, nullptr, owner, &cuts));
  };
  add_lane("A", {47, 59, 68}, false);
  add_lane("B", {53, 60, 74}, reverse);
  const auto section = v1::CreateRoadSectionDirect(fbb, "section", "motorLane", &lane_refs, nullptr, "road");

  std::vector<::flatbuffers::Offset<v1::RoadSection>> sections{section};
  if (reverse) {
    sections.push_back(v1::CreateRoadSectionDirect(fbb, "opposite", "motorLane", &opposite_refs, nullptr, "road"));
  }

  const auto road_alignment_rows = fbb.CreateVector(std::vector<::flatbuffers::Offset<v1::RoadAlignment>>{alignment});
  const auto road_corridor_rows = fbb.CreateVector(std::vector<::flatbuffers::Offset<v1::RoadCorridor>>{corridor});
  const auto road_section_rows = fbb.CreateVector(sections);
  const auto authoring_lane_rows = fbb.CreateVector(lanes);
  const auto lane_edge_rows = fbb.CreateVector(edges);
  const auto junction_rows = EmptyVectorOf<v1::Junction>(fbb);
  const auto movement_rows = EmptyVectorOf<v1::Movement>(fbb);
  const auto maneuver_path_rows = EmptyVectorOf<v1::ManeuverPath>(fbb);
  const auto maneuver_gate_rows = EmptyVectorOf<v1::ManeuverGate>(fbb);
  const auto waiting_zone_rows = EmptyVectorOf<v1::WaitingZone>(fbb);
  const auto stop_line_rows = EmptyVectorOf<v1::StopLine>(fbb);
  const auto signal_group_rows = EmptyVectorOf<v1::SignalGroup>(fbb);
  const auto signal_controller_rows = EmptyVectorOf<v1::SignalController>(fbb);
  const auto signal_phase_rows = EmptyVectorOf<v1::SignalPhase>(fbb);
  const auto parking_facility_rows = EmptyVectorOf<v1::ParkingFacility>(fbb);
  const auto parking_space_rows = EmptyVectorOf<v1::ParkingSpace>(fbb);
  const auto lane_group_rows = EmptyVectorOf<v1::LaneGroup>(fbb);
  const auto facility_band_rows = EmptyVectorOf<v1::FacilityBand>(fbb);
  const auto participant_class_rows = EmptyVectorOf<v1::ParticipantClass>(fbb);
  const auto access_rule_rows = EmptyVectorOf<v1::AccessRule>(fbb);
  const auto vehicle_profile_rows = EmptyVectorOf<v1::VehicleProfile>(fbb);
  const auto canonical_frame_rows = fbb.CreateVector(std::vector<::flatbuffers::Offset<v1::CanonicalFrame>>{frame});
  const auto conflict_zone_rows = EmptyVectorOf<v1::ConflictZone>(fbb);
  const auto participant_stream_rows = EmptyVectorOf<v1::ParticipantStream>(fbb);
  const auto conflict_zone_region_rows = EmptyVectorOf<v1::ConflictZoneRegion>(fbb);
  const auto right_of_way_policy_set_rows = EmptyVectorOf<v1::RightOfWayPolicySet>(fbb);
  const auto root = v1::CreateRoadEditingSource(
      fbb,
      /*format_version=*/6,
      header,
      v1::GeometryAccuracyProfile_Fine2Cm,
      v1::GeometryDirectionProfile_Smooth1Deg,
      road_alignment_rows,
      road_corridor_rows,
      road_section_rows,
      authoring_lane_rows,
      lane_edge_rows,
      junction_rows,
      movement_rows,
      maneuver_path_rows,
      maneuver_gate_rows,
      waiting_zone_rows,
      stop_line_rows,
      signal_group_rows,
      signal_controller_rows,
      signal_phase_rows,
      parking_facility_rows,
      parking_space_rows,
      lane_group_rows,
      facility_band_rows,
      participant_class_rows,
      access_rule_rows,
      vehicle_profile_rows,
      canonical_frame_rows,
      conflict_zone_rows,
      participant_stream_rows,
      conflict_zone_region_rows,
      right_of_way_policy_set_rows);

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
