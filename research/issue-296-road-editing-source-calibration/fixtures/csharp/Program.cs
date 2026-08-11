using System;
using System.IO;
using Google.FlatBuffers;
using LaneFlow.RoadEditing.V1;

internal static class Program
{
    private static int Main(string[] arguments)
    {
        if (arguments.Length != 1)
        {
            return 2;
        }

        var builder = new FlatBufferBuilder(1024);
        var frameKey = builder.CreateString("frame-main");
        var frame = CanonicalFrame.CreateCanonicalFrame(builder, frameKey);

        var roadAlignments = RoadEditingSource.CreateRoadAlignmentsVector(builder, Array.Empty<Offset<RoadAlignment>>());
        var roadCorridors = RoadEditingSource.CreateRoadCorridorsVector(builder, Array.Empty<Offset<RoadCorridor>>());
        var roadSections = RoadEditingSource.CreateRoadSectionsVector(builder, Array.Empty<Offset<RoadSection>>());
        var authoringLanes = RoadEditingSource.CreateAuthoringLanesVector(builder, Array.Empty<Offset<AuthoringLane>>());
        var laneEdges = RoadEditingSource.CreateLaneEdgesVector(builder, Array.Empty<Offset<LaneEdge>>());
        var junctions = RoadEditingSource.CreateJunctionsVector(builder, Array.Empty<Offset<Junction>>());
        var movements = RoadEditingSource.CreateMovementsVector(builder, Array.Empty<Offset<Movement>>());
        var maneuverPaths = RoadEditingSource.CreateManeuverPathsVector(builder, Array.Empty<Offset<ManeuverPath>>());
        var maneuverGates = RoadEditingSource.CreateManeuverGatesVector(builder, Array.Empty<Offset<ManeuverGate>>());
        var waitingZones = RoadEditingSource.CreateWaitingZonesVector(builder, Array.Empty<Offset<WaitingZone>>());
        var stopLines = RoadEditingSource.CreateStopLinesVector(builder, Array.Empty<Offset<StopLine>>());
        var signalGroups = RoadEditingSource.CreateSignalGroupsVector(builder, Array.Empty<Offset<SignalGroup>>());
        var signalControllers = RoadEditingSource.CreateSignalControllersVector(builder, Array.Empty<Offset<SignalController>>());
        var signalPhases = RoadEditingSource.CreateSignalPhasesVector(builder, Array.Empty<Offset<SignalPhase>>());
        var parkingAreas = RoadEditingSource.CreateParkingAreasVector(builder, Array.Empty<Offset<ParkingArea>>());
        var parkingSpaces = RoadEditingSource.CreateParkingSpacesVector(builder, Array.Empty<Offset<ParkingSpace>>());
        var laneGroups = RoadEditingSource.CreateLaneGroupsVector(builder, Array.Empty<Offset<LaneGroup>>());
        var facilityBands = RoadEditingSource.CreateFacilityBandsVector(builder, Array.Empty<Offset<FacilityBand>>());
        var participantClasses = RoadEditingSource.CreateParticipantClassesVector(builder, Array.Empty<Offset<ParticipantClass>>());
        var accessRules = RoadEditingSource.CreateAccessRulesVector(builder, Array.Empty<Offset<AccessRule>>());
        var vehicleProfiles = RoadEditingSource.CreateVehicleProfilesVector(builder, Array.Empty<Offset<VehicleProfile>>());
        var staticRoutes = RoadEditingSource.CreateStaticRoutesVector(builder, Array.Empty<Offset<StaticRoute>>());
        var canonicalFrames = RoadEditingSource.CreateCanonicalFramesVector(builder, new[] { frame });

        var generatorBuildId = builder.CreateString("cross-language-writer-v1");
        var description = builder.CreateString("cross-language fixture");
        Provenance.StartProvenance(builder);
        Provenance.AddDescription(builder, description);
        Provenance.AddFrontendOptionsDigest(builder, Digest256.CreateDigest256(builder, new byte[32]));
        Provenance.AddParametersAndInputsDigest(builder, Digest256.CreateDigest256(builder, new byte[32]));
        Provenance.AddGeneratorBuildId(builder, generatorBuildId);
        Provenance.AddKind(builder, ProvenanceKind.Generated);
        var provenance = Provenance.EndProvenance(builder);

        var moduleNamespace = builder.CreateString("cross-language");
        var sourceDocument = builder.CreateString("cross-language");
        var imports = ModuleHeader.CreateImportsVector(builder, Array.Empty<StringOffset>());
        var moduleHeader = ModuleHeader.CreateModuleHeader(
            builder,
            moduleNamespace,
            sourceDocument,
            imports,
            provenance);

        var root = RoadEditingSource.CreateRoadEditingSource(
            builder,
            1,
            moduleHeader,
            GeometryAccuracyProfile.Balanced5Cm,
            GeometryDirectionProfile.Balanced2Deg,
            roadAlignments,
            roadCorridors,
            roadSections,
            authoringLanes,
            laneEdges,
            junctions,
            movements,
            maneuverPaths,
            maneuverGates,
            waitingZones,
            stopLines,
            signalGroups,
            signalControllers,
            signalPhases,
            parkingAreas,
            parkingSpaces,
            laneGroups,
            facilityBands,
            participantClasses,
            accessRules,
            vehicleProfiles,
            staticRoutes,
            canonicalFrames);
        RoadEditingSource.FinishSizePrefixedRoadEditingSourceBuffer(builder, root);
        File.WriteAllBytes(arguments[0], builder.SizedByteArray());
        return 0;
    }
}
