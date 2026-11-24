// 最小跨语言 LFRE writer（C#）。构造固定最小模块并写出 size-prefixed bytes。
// 用途：证明非 Rust writer 产出的 bytes 能被生产 reader 接受（golden fixture）。
// 再生成步骤与钉版来源见 tools/lfre-crosslang-writer/README.md。
using System;
using System.IO;
using Google.FlatBuffers;
using LaneFlow.RoadEditing.V1;

internal static class Program
{
    // 与 Rust RoadEditingProvenance::direct 的冻结值一致
    // （crates/laneflow-compiler/src/road_editing/model.rs）。
    private static readonly byte[] InputsDigest =
    {
        0x6b, 0x27, 0xd0, 0xf7, 0x66, 0x93, 0xbc, 0xd3, 0x86, 0xac, 0x13, 0xdf, 0x72, 0x4e, 0x30, 0xf5,
        0xfb, 0x5a, 0xd3, 0xb9, 0xa1, 0x52, 0xa5, 0xe1, 0xf8, 0x8d, 0xe1, 0xa6, 0x24, 0xce, 0xa8, 0xaa,
    };

    private static readonly byte[] FrontendOptionsDigest =
    {
        0xb1, 0x62, 0x1e, 0x4a, 0x2d, 0xb8, 0xd7, 0x17, 0xb6, 0x50, 0x6b, 0x0a, 0xfb, 0x6f, 0xef, 0x5b,
        0xd4, 0xd5, 0x15, 0x6e, 0xcf, 0xe8, 0x87, 0xc5, 0xab, 0xf3, 0x6d, 0x08, 0x86, 0x9c, 0x78, 0x92,
    };

    private static int Main(string[] args)
    {
        bool reverse = args.Length == 2 && args[1] == "--reverse";
        if (args.Length != 1 && !reverse)
        {
            Console.Error.WriteLine("usage: CrosslangWriter <output.lfre> [--reverse]");
            return 1;
        }

        var builder = new FlatBufferBuilder(256);

        var buildId = builder.CreateString("laneflow-road-editing-direct-v1");
        var description = builder.CreateString("cross-language writer fixture");
        Provenance.StartProvenance(builder);
        Provenance.AddKind(builder, ProvenanceKind.Direct);
        Provenance.AddGeneratorBuildId(builder, buildId);
        Provenance.AddParametersAndInputsDigest(builder, Digest256.CreateDigest256(builder, InputsDigest));
        Provenance.AddFrontendOptionsDigest(builder, Digest256.CreateDigest256(builder, FrontendOptionsDigest));
        Provenance.AddDescription(builder, description);
        var provenance = Provenance.EndProvenance(builder);

        var authoringNamespace = builder.CreateString("city");
        var documentKey = builder.CreateString("roads/crosslang-writer");
        var imports = ModuleHeader.CreateImportsVector(builder, new StringOffset[0]);
        ModuleHeader.StartModuleHeader(builder);
        ModuleHeader.AddAuthoringNamespaceId(builder, authoringNamespace);
        ModuleHeader.AddSourceDocumentKey(builder, documentKey);
        ModuleHeader.AddImports(builder, imports);
        ModuleHeader.AddProvenance(builder, provenance);
        var header = ModuleHeader.EndModuleHeader(builder);

        var frameKey = builder.CreateString("frame");
        CanonicalFrame.StartCanonicalFrame(builder);
        CanonicalFrame.AddCanonicalFrameKey(builder, frameKey);
        var frame = CanonicalFrame.EndCanonicalFrame(builder);

        const double grade = 40.0 / 399.0;
        LineSegment.StartLineSegment(builder);
        LineSegment.AddEnd(builder, Vec3F64.CreateVec3F64(builder, 0.0, 40.0 * grade, 40.0));
        var line = LineSegment.EndLineSegment(builder);
        CurveSegment.StartCurveSegment(builder);
        CurveSegment.AddGeometryType(builder, CurveSegmentGeometry.LineSegment);
        CurveSegment.AddGeometry(builder, line.Value);
        var segment = CurveSegment.EndCurveSegment(builder);
        var spiral = ClothoidSegment.CreateClothoidSegment(builder, 16.0, 1.0 / 35.0, 0.0, grade);
        CurveSegment.StartCurveSegment(builder);
        CurveSegment.AddGeometryType(builder, CurveSegmentGeometry.ClothoidSegment);
        CurveSegment.AddGeometry(builder, spiral.Value);
        CurveSegment.AddSmoothEntry(builder, true);
        var transition = CurveSegment.EndCurveSegment(builder);

        var arc = CircularArcSegment.CreateCircularArcSegment(builder, 45.0, 35.0, grade);
        CurveSegment.StartCurveSegment(builder);
        CurveSegment.AddGeometryType(builder, CurveSegmentGeometry.CircularArcSegment);
        CurveSegment.AddGeometry(builder, arc.Value);
        var bend = CurveSegment.EndCurveSegment(builder);
        var segments = CurveProgram.CreateSegmentsVector(builder, new[] { segment, bend, transition });
        CurveProgram.StartCurveProgram(builder);
        CurveProgram.AddStart(builder, Vec3F64.CreateVec3F64(builder, 0.0, 0.0, 0.0));
        CurveProgram.AddSegments(builder, segments);
        var geometry = CurveProgram.EndCurveProgram(builder);

        var canonicalFrames = RoadEditingSource.CreateCanonicalFramesVector(builder, new[] { frame });

        // 向量必须在表构造开始前建好。
        var alignmentKey = builder.CreateString("alignment");
        var alignment = RoadAlignment.CreateRoadAlignment(builder, alignmentKey, frameKey, geometry);
        var roadKey = builder.CreateString("road");
        var sectionRef = builder.CreateString("road>section");
        var outerSection = reverse ? builder.CreateString("road>opposite") : sectionRef;
        var edges = new System.Collections.Generic.List<Offset<LaneEdge>>();
        Offset<AuthoringLane> AddLane(string key, double[] cuts, bool backward)
        {
            if (backward) Array.Reverse(cuts);
            var chain = new StringOffset[cuts.Length + 1];
            for (int i = 0; i < chain.Length; ++i)
            {
                int edgeIndex = backward ? cuts.Length - i : i;
                chain[i] = builder.CreateString($"{key}-{edgeIndex}");
                var next = i < cuts.Length
                    ? new[] { builder.CreateString($"{key}-{(backward ? edgeIndex - 1 : edgeIndex + 1)}") }
                    : Array.Empty<StringOffset>();
                var successors = LaneEdge.CreateSuccessorsVector(builder, next);
                edges.Add(LaneEdge.CreateLaneEdge(builder, chain[i], 10.0, successors));
            }
            var edgeChain = AuthoringLane.CreateEdgeChainVector(builder, chain);
            var stations = AuthoringLane.CreateInteriorCutStationsMetersVector(builder,
                Array.ConvertAll(cuts, station => station * 401.0 / 399.0));
            var laneKey = builder.CreateString(key);
            AuthoringLane.StartAuthoringLane(builder);
            AuthoringLane.AddAuthoringLaneKey(builder, laneKey);
            AuthoringLane.AddEdgeChain(builder, edgeChain);
            AuthoringLane.AddInteriorCutStationsMeters(builder, stations);
            AuthoringLane.AddDirection(builder, backward ? LaneDirection.Backward : LaneDirection.Forward);
            AuthoringLane.AddRoadSection(builder, backward ? outerSection : sectionRef);
            AuthoringLane.AddWidthProfile(builder, LinearWidthProfile.CreateLinearWidthProfile(builder, 3.5, 3.5));
            return AuthoringLane.EndAuthoringLane(builder);
        }
        var lanes = new[] { AddLane("A", new double[] {47, 59, 68}, false), AddLane("B", new double[] {53, 60, 74}, reverse) };
        var laneEdges = RoadEditingSource.CreateLaneEdgesVector(builder, edges.ToArray());
        var laneRef = builder.CreateString("road>section>A");
        var element = CorridorElement.CreateCorridorElement(builder, CorridorElementKind.RoadSection, sectionRef);
        var members = reverse ? new[] { element, CorridorElement.CreateCorridorElement(builder, CorridorElementKind.RoadSection, outerSection) } : new[] { element };
        var elements = RoadCorridor.CreateElementsVector(builder, members);
        var corridor = RoadCorridor.CreateRoadCorridor(builder, roadKey, alignmentKey, 0.0,
            StationEndKind.AlignmentEnd, 0.0, sectionRef, laneRef, elements);
        var laneRefs = RoadSection.CreateAuthoringLanesVector(builder, reverse ? new[] { laneRef } : new[] { laneRef, builder.CreateString("road>section>B") });
        var section = RoadSection.CreateRoadSection(builder, builder.CreateString("section"),
            builder.CreateString("motorLane"), laneRefs, default, roadKey);

        var sections = new System.Collections.Generic.List<Offset<RoadSection>> { section };
        if (reverse) {
            var outerLane = RoadSection.CreateAuthoringLanesVector(builder, new[] { builder.CreateString("road>opposite>B") });
            sections.Add(RoadSection.CreateRoadSection(builder, builder.CreateString("opposite"),
                builder.CreateString("motorLane"), outerLane, default, roadKey));
        }

        var roadAlignments = RoadEditingSource.CreateRoadAlignmentsVector(builder, new[] { alignment });
        var roadCorridors = RoadEditingSource.CreateRoadCorridorsVector(builder, new[] { corridor });
        var roadSections = RoadEditingSource.CreateRoadSectionsVector(builder, sections.ToArray());
        var authoringLanes = RoadEditingSource.CreateAuthoringLanesVector(builder, lanes);
        var junctions = EmptyVector<Junction>(builder);
        var movements = EmptyVector<Movement>(builder);
        var maneuverPaths = EmptyVector<ManeuverPath>(builder);
        var maneuverGates = EmptyVector<ManeuverGate>(builder);
        var waitingZones = EmptyVector<WaitingZone>(builder);
        var stopLines = EmptyVector<StopLine>(builder);
        var signalGroups = EmptyVector<SignalGroup>(builder);
        var signalControllers = EmptyVector<SignalController>(builder);
        var signalPhases = EmptyVector<SignalPhase>(builder);
        var parkingFacilities = EmptyVector<ParkingFacility>(builder);
        var parkingSpaces = EmptyVector<ParkingSpace>(builder);
        var laneGroups = EmptyVector<LaneGroup>(builder);
        var facilityBands = EmptyVector<FacilityBand>(builder);
        var participantClasses = EmptyVector<ParticipantClass>(builder);
        var accessRules = EmptyVector<AccessRule>(builder);
        var vehicleProfiles = EmptyVector<VehicleProfile>(builder);
        var conflictZones = EmptyVector<ConflictZone>(builder);
        var participantStreams = EmptyVector<ParticipantStream>(builder);
        var conflictZoneRegions = EmptyVector<ConflictZoneRegion>(builder);
        var rightOfWayPolicySets = EmptyVector<RightOfWayPolicySet>(builder);

        RoadEditingSource.StartRoadEditingSource(builder);
        RoadEditingSource.AddFormatVersion(builder, 6u);
        RoadEditingSource.AddModuleHeader(builder, header);
        RoadEditingSource.AddGeometryAccuracyProfile(builder, GeometryAccuracyProfile.Fine2Cm);
        RoadEditingSource.AddGeometryDirectionProfile(builder, GeometryDirectionProfile.Smooth1Deg);
        RoadEditingSource.AddRoadAlignments(builder, roadAlignments);
        RoadEditingSource.AddRoadCorridors(builder, roadCorridors);
        RoadEditingSource.AddRoadSections(builder, roadSections);
        RoadEditingSource.AddAuthoringLanes(builder, authoringLanes);
        RoadEditingSource.AddLaneEdges(builder, laneEdges);
        RoadEditingSource.AddJunctions(builder, junctions);
        RoadEditingSource.AddMovements(builder, movements);
        RoadEditingSource.AddManeuverPaths(builder, maneuverPaths);
        RoadEditingSource.AddManeuverGates(builder, maneuverGates);
        RoadEditingSource.AddWaitingZones(builder, waitingZones);
        RoadEditingSource.AddStopLines(builder, stopLines);
        RoadEditingSource.AddSignalGroups(builder, signalGroups);
        RoadEditingSource.AddSignalControllers(builder, signalControllers);
        RoadEditingSource.AddSignalPhases(builder, signalPhases);
        RoadEditingSource.AddParkingFacilities(builder, parkingFacilities);
        RoadEditingSource.AddParkingSpaces(builder, parkingSpaces);
        RoadEditingSource.AddLaneGroups(builder, laneGroups);
        RoadEditingSource.AddFacilityBands(builder, facilityBands);
        RoadEditingSource.AddParticipantClasses(builder, participantClasses);
        RoadEditingSource.AddAccessRules(builder, accessRules);
        RoadEditingSource.AddVehicleProfiles(builder, vehicleProfiles);
        RoadEditingSource.AddCanonicalFrames(builder, canonicalFrames);
        RoadEditingSource.AddConflictZones(builder, conflictZones);
        RoadEditingSource.AddParticipantStreams(builder, participantStreams);
        RoadEditingSource.AddConflictZoneRegions(builder, conflictZoneRegions);
        RoadEditingSource.AddRightOfWayPolicySets(builder, rightOfWayPolicySets);
        var root = RoadEditingSource.EndRoadEditingSource(builder);

        RoadEditingSource.FinishSizePrefixedRoadEditingSourceBuffer(builder, root);
        var bytes = builder.SizedByteArray();
        File.WriteAllBytes(args[0], bytes);
        Console.Out.WriteLine($"wrote {bytes.Length} bytes to {args[0]}");
        return 0;
    }

    private static VectorOffset EmptyVector<T>(FlatBufferBuilder builder)
        where T : struct, IFlatbufferObject
    {
        builder.StartVector(4, 0, 4);
        return builder.EndVector();
    }
}
