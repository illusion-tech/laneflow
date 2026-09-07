use super::*;

fn anchor(edge: String, progress_mm: u32) -> ParkingAnchor {
    ParkingAnchor { edge, progress_mm }
}

fn input(anchor: &ParkingAnchor) -> Result<re::ParkingLaneAnchor> {
    Ok(re::ParkingLaneAnchor::try_new(
        edge_ref(&anchor.edge)?,
        f64::from(anchor.progress_mm) / 1_000.0,
    )?)
}

pub(super) fn add(
    builder: &mut Builder<'_>,
    config: &UrbanConfig,
    layout: &Layout,
) -> Result<Vec<ParkingPool>> {
    let mut pools = Vec::new();
    for cell in &layout.cells {
        let arm = if cell.slot % 2 == 0 {
            Direction::East
        } else {
            Direction::West
        };
        let key = format!("{}.mixed", cell.key());
        let entry = anchor(cell.edge_key(arm, false), 40_000);
        let exit = anchor(cell.edge_key(arm, true), 40_000);
        builder.add_declaration(re::RoadEditingDeclaration::ParkingFacility(
            re::ParkingFacilityInput::try_new(&key)?.with_virtual_capacity(
                config.cell_virtual_capacity,
                vec![input(&entry)?],
                vec![input(&exit)?],
            ),
        ))?;
        pools.push(ParkingPool {
            key: key.clone(),
            facility: key.clone(),
            tile: cell.tile,
            virtual_pool: true,
            capacity: config.cell_virtual_capacity,
            entries: vec![entry.clone()],
            exits: vec![exit],
        });
        // Two distinct visible spaces make every mixed facility exercise the plural-bay case.
        for (index, (entry_progress, exit_progress)) in
            [(40_000, 60_000), (70_000, 85_000)].into_iter().enumerate()
        {
            let bay = format!("{}.bay{index}", cell.key());
            let bay_entry = anchor(entry.edge.clone(), entry_progress);
            let bay_exit = anchor(entry.edge.clone(), exit_progress);
            builder.add_declaration(re::RoadEditingDeclaration::ParkingSpace(
                re::ParkingSpaceInput::try_new(
                    &bay,
                    input(&bay_entry)?,
                    input(&bay_exit)?,
                    re::ParkingSpaceGeometry::try_new(
                        -4.0,
                        0.0,
                        config.parking_space_length_meters,
                        2.5,
                    )?,
                )?
                .with_parking_facility(re::ParkingFacilityReference::local(&key)?),
            ))?;
            pools.push(ParkingPool {
                key: bay,
                facility: key.clone(),
                tile: cell.tile,
                virtual_pool: false,
                capacity: 1,
                entries: vec![bay_entry],
                exits: vec![bay_exit],
            });
        }
    }
    for tile in 0..layout.scale.tile_count() {
        let key = format!("t{tile:03}.underground");
        let mut entries = Vec::new();
        let mut exits = Vec::new();
        for (slot, arm) in [(8, Direction::East), (9, Direction::West)] {
            let cell = &layout.cells[(tile * 10 + slot) as usize];
            entries.push(anchor(cell.edge_key(arm, false), 40_000));
            exits.push(anchor(cell.edge_key(arm, true), 40_000));
        }
        builder.add_declaration(re::RoadEditingDeclaration::ParkingFacility(
            re::ParkingFacilityInput::try_new(&key)?.with_virtual_capacity(
                config.garage_virtual_capacity,
                entries.iter().map(input).collect::<Result<_>>()?,
                exits.iter().map(input).collect::<Result<_>>()?,
            ),
        ))?;
        pools.push(ParkingPool {
            key: key.clone(),
            facility: key,
            tile,
            virtual_pool: true,
            capacity: config.garage_virtual_capacity,
            entries,
            exits,
        });
    }
    Ok(pools)
}
