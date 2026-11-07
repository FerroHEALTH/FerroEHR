## Technische specificatie

Deze sectie beschrijft de NEN7513-velden zoals verplicht gesteld en hoe die verwerkt worden in een logboek.
Sommige van deze velden vloeien voort uit attributen beschreven in het Logboek Dataverwerkingen standaard zelf.
Alle missende velden komen voor in `resource` of `attributes` uit de interface van het Logboek component.

### Reeds beschreven NEN-velden in Logboek Dataverwerkingen

Het Logboek component beschrijft in de interface al enkele velden die mappen naar [[NEN7513]]-velden.
Deze mapping is als volgt:

| Logboek veldnaam | NEN7513-veldnaam                            |
|------------------|---------------------------------------------|
| `trace_id`       | Onderdeel van W3C-Trace-Context-Traceparent |
| `span_id`        | Onderdeel van W3C-Trace-Context-Traceparent |
| `status_code`    | `EventOutcomeIndicator`                     |
| `name`           | `EventActionCode`                           |
| `start_time`     | `EventDateTime`                             |
| `parent_span_id` | Onderdeel van W3C-Trace-Context-Traceparent |

<p class="note">
Let op: `end_time` is ook onderdeel van de interface van het Logboek component, maar niet vereist voor de NEN7513-norm.
</p>

Tevens zijn er bestaande attributen in de namespace `dpl.core` die mappen naar NEN7513-velden:

| Core attribute                    | NEN7513-veldnaam              |
|-----------------------------------|-------------------------------|
| `dpl.core.processing_activity_id` | `EventID`                     |
| `dpl.core.data_subject_id`        | `ParticipantObjectID`         |
| `dpl.core.data_subject_id_type`   | `ParticipantObjectIDTypeCode` |

<p class="note">
Let op: `ParticipantObjectID` en `ParticipantObjectIDTypeCode` komen tevens respectievelijk voor als `dpl.nen7513.participantObjectId` en `dpl.nen7513.participantObjectIdType` attributes in de namespace-specifieke data zoals hieronder beschreven.
Dit is gedaan om alle NEN7513 velden in 1 object te plaatsen wat verder gebruik kan vergemakkelijken.
</p>

### Uitbreiding `resource` van Logboek component

Het `resource` veld in het Logboek component wordt uitgebreid met de volgende attributen: 

```
{
    "attributes": {
        "nen7513.AuditEnterpriseSiteID": 5,
        "nen7513.AuditSourceId": 42,
        "nen7513.AuditSourceTypeCode": "comp1",
    }
}
```

Hierbij is de namespace `nen7513` en komen alle attributen voor van de [[NEN7513]]-velden uit de categorie "bronsysteem".

## Namespace attributes

Alle overige [[NEN7513]]-velden zijn verwerkt in de `attributes` met namespace `dpl.nen7513`:

```
dpl.nen7513.eventTypeCode
dpl.nen7513.users: [
    {
        userID,
        alternateUserID,
        userName,
        userIsRequestor,
        roleIDCode,
        purposeOfUse,
        networkAccessPointTypeCode,
        networkAccessPointID,
    }
]
dpl.nen7513.participants: [
    {
        participantObjectTypeCode,
        participantObjectTypeCodeRole,
        participantObjectDataLifeCycle,
        participantObjectIDTypeCode,
        participantObjectPermissionPolicySet,
        participantObjectSensitivity,
        participantObjectID,
        participantObjectName,
        participantObjectDetail,
        participantObjectDescription,
    }
]
dpl.nen7513.participantSegments: [
    {
        participantObjectTypeCode,
        participantObjectTypeCodeRole,
        participantObjectDataLifeCycle,
        participantObjectIDTypeCode,
        participantObjectPermissionPolicySet,
        participantObjectSensitivity,
        participantObjectID,
        participantObjectName,
        participantObjectDetail,
        participantObjectDescription,
    }
]
dpl.nen7513.autorisatieProtocol
dpl.nen7513.behandelrelatieProtocol
dpl.nen7513.toestemmingsProfiel
dpl.nen7513.controles
```
