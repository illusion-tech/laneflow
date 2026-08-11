#include "road-editing_generated.h"

#include <cstdint>
#include <fstream>
#include <string>
#include <vector>

namespace wire = LaneFlow::RoadEditing::V1;

int main(int argc, char **argv) {
  if (argc != 2) {
    return 2;
  }

  flatbuffers::FlatBufferBuilder builder(1024);
  const wire::Digest256 zero_digest;
  const auto provenance = wire::CreateProvenanceDirect(
      builder, wire::ProvenanceKind_Generated, "cross-language-writer-v1",
      &zero_digest, &zero_digest, nullptr, "cross-language fixture");
  const std::vector<flatbuffers::Offset<flatbuffers::String>> imports;
  const auto module_header = wire::CreateModuleHeaderDirect(
      builder, "cross-language", "cross-language", &imports, provenance);
  const auto frame =
      wire::CreateCanonicalFrameDirect(builder, "frame-main", nullptr);

  const std::vector<flatbuffers::Offset<wire::RoadAlignment>> road_alignments;
  const std::vector<flatbuffers::Offset<wire::RoadCorridor>> road_corridors;
  const std::vector<flatbuffers::Offset<wire::RoadSection>> road_sections;
  const std::vector<flatbuffers::Offset<wire::AuthoringLane>> authoring_lanes;
  const std::vector<flatbuffers::Offset<wire::LaneEdge>> lane_edges;
  const std::vector<flatbuffers::Offset<wire::Junction>> junctions;
  const std::vector<flatbuffers::Offset<wire::Movement>> movements;
  const std::vector<flatbuffers::Offset<wire::ManeuverPath>> maneuver_paths;
  const std::vector<flatbuffers::Offset<wire::ManeuverGate>> maneuver_gates;
  const std::vector<flatbuffers::Offset<wire::WaitingZone>> waiting_zones;
  const std::vector<flatbuffers::Offset<wire::StopLine>> stop_lines;
  const std::vector<flatbuffers::Offset<wire::SignalGroup>> signal_groups;
  const std::vector<flatbuffers::Offset<wire::SignalController>>
      signal_controllers;
  const std::vector<flatbuffers::Offset<wire::SignalPhase>> signal_phases;
  const std::vector<flatbuffers::Offset<wire::ParkingArea>> parking_areas;
  const std::vector<flatbuffers::Offset<wire::ParkingSpace>> parking_spaces;
  const std::vector<flatbuffers::Offset<wire::LaneGroup>> lane_groups;
  const std::vector<flatbuffers::Offset<wire::FacilityBand>> facility_bands;
  const std::vector<flatbuffers::Offset<wire::ParticipantClass>>
      participant_classes;
  const std::vector<flatbuffers::Offset<wire::AccessRule>> access_rules;
  const std::vector<flatbuffers::Offset<wire::VehicleProfile>> vehicle_profiles;
  const std::vector<flatbuffers::Offset<wire::StaticRoute>> static_routes;
  const std::vector<flatbuffers::Offset<wire::CanonicalFrame>> canonical_frames{
      frame};

  const auto root = wire::CreateRoadEditingSourceDirect(
      builder, 1, module_header,
      wire::GeometryAccuracyProfile_Balanced5Cm,
      wire::GeometryDirectionProfile_Balanced2Deg, &road_alignments,
      &road_corridors, &road_sections, &authoring_lanes, &lane_edges,
      &junctions, &movements, &maneuver_paths, &maneuver_gates, &waiting_zones,
      &stop_lines, &signal_groups, &signal_controllers, &signal_phases,
      &parking_areas, &parking_spaces, &lane_groups, &facility_bands,
      &participant_classes, &access_rules, &vehicle_profiles, &static_routes,
      &canonical_frames);
  wire::FinishSizePrefixedRoadEditingSourceBuffer(builder, root);

  std::ofstream output(argv[1], std::ios::binary | std::ios::trunc);
  output.write(reinterpret_cast<const char *>(builder.GetBufferPointer()),
               static_cast<std::streamsize>(builder.GetSize()));
  return output.good() ? 0 : 3;
}
