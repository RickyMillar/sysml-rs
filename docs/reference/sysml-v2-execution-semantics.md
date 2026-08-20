# Comprehensive catalogue of SysML capabilities for tool implementation

**SysML v1 and v2 together define the most complete language for model-based systems engineering, spanning structural and behavioral architecture, requirements traceability, parametric analysis, and cross-domain integration.** With SysML v2 formally adopted by OMG in July 2025 (published September 2025), the landscape now includes a new KerML-grounded metamodel, dual textual/graphical notation, and a standardized REST/OSLC API — fundamentally expanding what tools can enable. This catalogue organizes every user-facing capability across three categories: the model itself, runtime/execution, and views/diagrams. Each item is phrased as what a user can *do*, making it directly convertible into user stories.

---

## 1. The model: what users can define, relate, and organize

### 1.1 Structural modeling

**Definitions and usages (the core pattern).** In v2, all modeling follows a consistent *definition/usage* pattern — users define reusable types and then instantiate them as usages in specific contexts. In v1, the equivalent is blocks and their properties.

- **Define part definitions** (v2) or **blocks** (v1) to represent system components with attributes (value properties), operations, receptions, and nested parts. A user can specify a block or part definition with typed properties, default values, multiplicity, and ordering constraints.
- **Create part usages** within owning contexts, establishing composition hierarchies. A user can decompose a system into subsystems, assemblies, and components to any depth.
- **Define and use attribute definitions/usages** for non-part data — quantities, measurements, configuration parameters — separate from structural parts.
- **Define value types** with units, quantity kinds, and dimensions. A user can create a type representing "mass in kilograms" with associated unit and dimension, referencing the ISO 80000 standard library or QUDV (Quantities, Units, Dimensions, Values).
- **Define enumerations** (enumeration definitions in v2) with literal values. A user can constrain a property to one of a fixed set of options.
- **Define and apply signal types** for asynchronous communication between components.

**Ports, interfaces, and connectors.** Ports define the interaction points of components.

- **Define port definitions/usages** (v2) or **flow ports, proxy ports, and full ports** (v1). A user can specify what items (signals, data, energy, matter) can flow through a port and in which direction (in, out, inout).
- **Define interface definitions** (v2) or **interface blocks** (v1) to specify conjugate port contracts — what one side provides and the other requires.
- **Create connectors** (connection usages in v2) linking ports between parts. Connectors can be assembly connectors (peer-to-peer), delegation connectors (outer port to inner part port), or binding connectors (equating properties).
- **Define items and item flows** to specify what flows along a connector — material, energy, data, or signals — including flow direction and typing.
- **Specify connection definitions** (v2) as reusable, typed connection patterns that can be instantiated across the model.

**Relationships and type hierarchies.** SysML supports a rich set of relationships to express architecture patterns.

- **Specialize (generalize)** one definition from another, inheriting all features. A user can create a "Sensor" definition and specialize it into "TemperatureSensor" and "PressureSensor."
- **Redefine** inherited features in a specialization to change type, multiplicity, or default value.
- **Subset** one feature from another, indicating that instances of the subsetting feature are a subset of instances of the subsetted feature.
- **Create associations and compositions** between definitions. Composition (filled diamond in v1) indicates ownership; reference associations indicate non-owning references.
- **Use feature chaining** (v2) to navigate multi-step paths through the model — e.g., `vehicle.engine.pistons` as a single feature reference.
- **Define dependency, realization, and abstraction** relationships for traceability between model elements at different levels of abstraction.

### 1.2 Behavioral modeling

**Actions and activities.** Activities model the functional behavior of a system.

- **Define action definitions/usages** (v2) or **activities with actions** (v1) specifying what a system or component does, with input/output parameters (pins in v1).
- **Compose actions using control flow** — succession (v2) or control flow edges (v1) to sequence actions. A user can create sequential, parallel (fork/join), conditional (decision/merge), and loop structures.
- **Use object flows** to pass items between actions — connecting output pins to input pins through object flow edges or flow connection usages (v2).
- **Define if-then-else actions, for-loop actions, while-loop actions** (v2) as first-class constructs, eliminating the need for opaque decision nodes.
- **Define accept actions** (accept event actions in v1) to wait for an event, signal, or time event before proceeding.
- **Define send actions** to asynchronously dispatch signals to target parts.
- **Define perform actions** (v2) to reference an action definition defined elsewhere and execute it within a given behavioral context.
- **Define assign actions** (v2) to modify a feature's value during execution.
- **Partition/swimlane actions** by responsible component using activity partitions (v1) or allocation (v2).
- **Define call behavior/operation actions** (v1) to invoke other activities or operations from within an activity.
- **Use structured activity nodes** (v1) to group related actions with their own local scope.

**State machines.** State machines model the discrete event-driven behavior of a component over time.

- **Define state definitions/usages** (v2) or **state machines** (v1) with states, transitions, triggers, guards, and effects.
- **Create composite (nested) states** containing substates, allowing hierarchical decomposition of behavior.
- **Define orthogonal/concurrent regions** within a state, modeling parallel behaviors that execute simultaneously.
- **Specify entry, exit, and do activities** on states — behaviors that execute on entering, leaving, or while residing in a state.
- **Use pseudostates**: initial, final, choice, junction, history (shallow and deep), fork, join, terminate — to model complex transition logic.
- **Define transitions with triggers** (signal reception, time event, change event), **guards** (boolean conditions), and **effects** (actions executed during transition).
- **Reference actions defined elsewhere** as transition effects or state behaviors, maintaining consistency between structural and behavioral views.

**Interactions and sequences.** Sequence-level behavior specifies the message exchange order between parts.

- **Define lifelines** representing part instances that participate in an interaction.
- **Create messages** (synchronous calls, asynchronous signals, replies, create/destroy) between lifelines with ordering constraints.
- **Use combined fragments**: alt (alternative), opt (optional), loop, break, par (parallel), critical, neg (negative/forbidden), assert, ignore, consider — to express complex interaction logic.
- **Create interaction references** to decompose large sequences into reusable sub-interactions.
- **Define gates** on interaction boundaries for connecting messages across interaction references.

**Use cases.** Use cases capture system capabilities from the user perspective.

- **Define use case definitions/usages** (v2) or **use cases** (v1) with actors (both human and system), system boundaries, and include/extend relationships.
- **Specify use case subjects** — the system or subsystem that the use case applies to.
- **Create include relationships** to factor out common behavior and **extend relationships** with extension points for optional/conditional behavior.

### 1.3 Requirements modeling

- **Define requirement definitions/usages** (v2) or **requirements** (v1) with text, ID, priority, risk, status, source, and custom attributes. Requirements in v2 are *constraints* — they have formal `assume` and `require` expressions that can be machine-evaluated.
- **Decompose requirements** into sub-requirements through containment, forming requirement hierarchies.
- **Establish satisfy relationships** linking design elements (parts, actions) to the requirements they satisfy.
- **Establish verify relationships** linking test cases or verification cases to the requirements they verify.
- **Establish derive relationships** between requirements at different levels of abstraction — stakeholder needs to system requirements to subsystem requirements.
- **Establish refine relationships** linking model elements that elaborate or detail a requirement.
- **Establish trace relationships** for general traceability between requirements and other artifacts.
- **Use copy relationships** (v1) to replicate requirements across packages while maintaining identity.
- **Define concerns and stakeholders** (v2) as first-class elements, linking who cares about what and why.
- **Define verification cases** (v2) — formalized test procedures that reference the requirement's constraint and can programmatically determine pass/fail.
- **Define analysis cases** (v2) to formalize trade study and analysis procedures as model elements, with specified objectives, inputs, and results.
- **Define calculation definitions/usages** (v2) to express mathematical computations as reusable, parameterized model elements — e.g., total mass = sum of part masses.

### 1.4 Parametric and analysis modeling

- **Define constraint blocks** (v1) or **constraint definitions** (v2) containing mathematical equations with parameters. A user can express `F = m × a` as a constraint block with parameters F, m, a.
- **Bind constraint parameters to part properties** using binding connectors in parametric diagrams, linking analysis equations to the system's value properties.
- **Compose multiple constraints** in a parametric network — connecting outputs of one constraint to inputs of another to build multi-step analysis chains.
- **Define analysis cases** (v2) with objective functions, input parameters, analysis steps, and result usages — formalizing the entire analysis workflow within the model.
- **Define trade study structures** evaluating multiple alternatives against weighted criteria, with the model capturing both the alternatives and the evaluation logic.

### 1.5 Allocation

- **Create allocate relationships** (v1) mapping logical functions to physical components — e.g., allocating an activity (behavior) to a block (structure).
- **Express logical-to-physical allocation** — mapping logical architecture elements to physical implementation elements.
- **Express function-to-component allocation** — assigning behavioral actions to structural parts responsible for executing them.
- **Express software-to-hardware allocation** — mapping software modules to processors/hardware nodes.
- **In v2, allocation is modeled through usage bindings and perform actions** rather than a dedicated allocation stereotype — a user allocates behavior to structure by defining that a part *performs* an action, achieving the same traceability with stronger semantic precision.

### 1.6 Package and namespace organization

- **Create packages** to organize model elements into namespaces — by domain, discipline, lifecycle phase, abstraction level, or any user-defined scheme.
- **Use package imports** (public and private) to control visibility of elements across packages.
- **Create model libraries** — reusable packages of definitions (value types, units, interface patterns, common blocks) shareable across projects.
- **Reference standard libraries**: SI units, ISO 80000 quantities, QUDV, and SysML v2's built-in library packages (ScalarValues, ISQ, SI, AnalysisCases, etc.).
- **Use namespace-qualified names** to unambiguously reference elements across packages (e.g., `VehicleLibrary::Engine::Piston`).

### 1.7 Extensibility and profiles

**SysML v1 extensibility:**
- **Define profiles** containing stereotypes that extend UML/SysML metaclasses with additional tagged values and constraints.
- **Apply stereotypes** to model elements to add domain-specific semantics — e.g., «safety-critical», «COTS», «ASIL-D».
- **Define OCL constraints** on stereotypes to enforce domain rules.

**SysML v2 extensibility:**
- **Define metadata definitions** (replacing stereotypes) as first-class model elements, which are themselves typed and can carry structured data.
- **Annotate any model element with metadata** to add domain-specific properties without modifying the core metamodel.
- **Define language extensions** using KerML's extensibility mechanisms, creating domain-specific languages (DSLs) on top of SysML v2.
- **Use the textual notation** to author models in a code-like syntax (`part def`, `action def`, `requirement def`, etc.), enabling version control, diff/merge, and automation workflows that are impossible with purely graphical models.

### 1.8 Additional v2 constructs

- **Occurrence definitions/usages** — the abstract supertype for parts, actions, states, and other things that happen or exist over time, enabling lifecycle modeling.
- **Individuals** vs. **types** — a user can model both the class of a thing (e.g., "TemperatureSensor" as a type) and a specific individual instance (e.g., "sensor-SN-4472"), with snapshots and time slices capturing the individual's state at specific moments.
- **Variant modeling** — a user can define variation points and variant configurations within the model to represent product-line variability.
- **Expose/filter mechanisms** on views/viewpoints for controlling what subset of the model is rendered or exported.
- **Comment, documentation, and rationale elements** — a user can annotate any element with free-text comments, documentation, or rationale for design decisions.

---

## 2. Runtime and execution: what users can simulate, analyze, and integrate

### 2.1 Model execution and simulation

**Activity/action execution.** Tools implement the OMG fUML (Foundational UML) standard to execute activity models.

- **Execute activity diagrams** step-by-step with token-based semantics — tokens flow through control and object flows, actions fire when input tokens arrive. Users can observe execution state, pause, step, and resume.
- **Animate behavioral diagrams** during execution — highlighted current action, flowing tokens, and runtime values displayed on diagram elements in real time.
- **Execute opaque action bodies** written in expression languages (Alf for fUML, JavaScript, Groovy, or tool-specific languages) within the model execution environment.
- **Simulate time-dependent behaviors** with simulated clocks, time events, and after/when time triggers.

**State machine execution.** Implements OMG PSSM (Precise Semantics of State Machines).

- **Execute state machines** — inject events (signals, time events, change events), observe transitions firing, monitor the current active state configuration including nested and orthogonal regions.
- **Step through transitions** with animation, displaying guard evaluation results and effect execution.
- **Validate state machine completeness** — detect unreachable states, missing transitions, deadlocks, or livelocks through execution exploration.

**Composite structure execution.** Implements OMG PSCS (Precise Semantics of Composite Structures).

- **Execute systems of interacting parts** — instantiate blocks/parts, route signals through ports and connectors, and observe end-to-end system behavior across components.
- **Inject stimuli** (signals, calls) at system boundaries and observe how they propagate through the internal structure.
- **Capture execution traces** (sequence of messages, state transitions, action firings) as interaction traces that can be compared against expected sequences.

### 2.2 Parametric constraint solving and analysis

- **Solve parametric constraint networks** — given known values, compute unknown values by propagating constraints. For example, given mass and acceleration, solve for force.
- **Run Monte Carlo analysis** on parametric models — apply statistical distributions (uniform, normal, log-normal, etc.) to input parameters, run thousands of iterations, and collect output distributions for sensitivity analysis. Cameo Simulation Toolkit natively supports this with automated statistics (mean, deviation, out-of-spec percentage) and CSV export.
- **Run trade studies** — define alternatives (design configurations), evaluation criteria (weighted objectives), and use the parametric engine to score and rank alternatives. Cameo evaluates all permutations of parametric and design alternatives.
- **Compute rollups** — recursively calculate aggregate properties (total mass, power budget, cost) up the containment hierarchy and check results against constraints.
- **Verify parametric constraints against requirements** — automatically determine whether computed values satisfy linked requirements and display pass/fail verification status.

### 2.3 Co-simulation and external tool integration

- **Integrate with MATLAB/Simulink** — export SysML block structures to Simulink models, synchronize parameters bidirectionally, and run coupled simulations where SysML orchestrates high-level behavior while Simulink executes detailed continuous-time dynamics. Intercax Syndeia and Cameo DataHub both support this integration.
- **Integrate with Modelica** — map SysML parametric models to Modelica equation systems for continuous-time physical simulation (thermal, electrical, mechanical, fluid dynamics). Cameo and Rhapsody support Modelica export/import.
- **Use FMI/FMU (Functional Mock-up Interface)** — export SysML models as FMUs or import external FMUs to co-simulate SysML behavioral models with physics engines, control systems, or other domain tools. **IBM Rhapsody explicitly supports FMI/FMU co-simulation**, and Cameo supports SSP (System Structure and Parameterization) file export for tool-to-tool model exchange (e.g., with Dymola).
- **Connect to PLM systems** (Teamcenter, Windchill, 3DEXPERIENCE) — synchronize system architecture models with product structure, BOMs, and configuration data.
- **Connect to requirements management tools** (IBM DOORS/DOORS Next, Jama Connect, Polarion) — bidirectionally sync requirements between the SysML model and the requirements repository, maintaining traceability links.
- **Connect to ALM/issue tracking** (Jira, GitHub, Azure DevOps) — link system design elements to work items, user stories, and software tasks.
- **Connect to CAD systems** (NX, Creo, CATIA) — push design parameters from SysML to CAD and pull back geometric data and physical properties.
- **Use Intercax Syndeia** as a dedicated digital thread platform federating SysML models with PLM, ALM, CAD, simulation, requirements, and test management tools via drag-and-drop transformations, bidirectional sync, and cross-tool traceability visualization.

### 2.4 API-based model access

**SysML v2 standard API (Systems Modeling API and Services 1.0):**

- **CRUD operations on model elements** — programmatically create, read, update, and delete any element in a SysML v2 model through a standard REST/HTTP API with JSON payloads.
- **Navigate model structure** — traverse containment, membership, specialization, and relationship hierarchies via API calls.
- **Query models** — execute queries against the model repository to find elements matching criteria (by type, name, relationship, metadata).
- **Manage projects and commits** — the API defines project, branch, and commit concepts, enabling version-controlled model management through API calls.
- **Export/import model content** — publish models from Jupyter/Eclipse editors to a repository server, and retrieve models from the repository.
- **OSLC binding** — the API includes an OSLC Platform-Specific Model (PSM), enabling SysML v2 resources to integrate with the broader OSLC lifecycle ecosystem (linking to OSLC requirements, change requests, test cases, and architecture resources).

**Tool-specific APIs:**
- **Cameo/MagicDraw OpenAPI** — full programmatic access to the model via Java API, plus scripting in Jython, Groovy, JavaScript, and BeanShell for custom automations, macros, and plugins.
- **IBM Rhapsody API** — Java/COM-based API for model manipulation, plus JavaScript/Python extensions in Rhapsody SE.
- **Sparx Enterprise Architect Automation Interface** — COM/.NET API exposing all model elements, diagrams, and features for programmatic access. Trechoro extends this with direct scripting access to SysML v2 abstract syntax structures.
- **Eclipse Papyrus** — Eclipse Modeling Framework (EMF) API for full programmatic model access, plus Java plugin extensibility.

### 2.5 Model validation and consistency checking

- **Run well-formedness checks** — validate that the model conforms to SysML metamodel rules (correct element types, valid relationships, required properties present).
- **Execute OCL constraints** — evaluate Object Constraint Language expressions defined on stereotypes, profiles, or model elements to enforce custom rules.
- **Run completeness checks** — identify missing elements (ports without types, blocks without properties, requirements without satisfy links, states without transitions).
- **Run custom validation rules** — define project- or organization-specific rules (naming conventions, mandatory stereotypes, required traceability) and check models against them.
- **Detect and report errors** with navigation to the offending element — tools highlight issues in the containment tree, provide error descriptions, and suggest fixes.
- **Verify requirements against design** — automatically check whether satisfy, verify, derive, and refine relationships are complete and consistent.
- **Static model checking** (Rhapsody) — analyze model consistency and completeness before execution, identifying structural issues.

### 2.6 Model transformation and code generation

- **Generate source code** from models — IBM Rhapsody generates production code in **C, C++, Java, and Ada** from UML/SysML models, including state machine implementations and class structures. Code generation is TÜV-certified for safety-critical standards (ISO 26262, DO-178C).
- **Round-trip engineering** — synchronize changes between model and code in both directions, keeping the model and implementation aligned (Rhapsody).
- **Generate documents** from models — produce system specifications, design documents, interface control documents, and reports in Word, PDF, or HTML format using customizable templates. Cameo's Report Wizard and DocGen, Sparx EA's document generator, and Rhapsody's reporting engine all support this.
- **Generate web-based model publications** — publish read-only interactive model views to web portals for stakeholder review without requiring a tool license (Cameo Collaborator, Sparx Prolaborate).
- **Transform SysML v1 to SysML v2** — OMG has published a normative transformation specification (`SysML/2.0/Transformation`) defining mapping rules for migrating v1 models to v2 with semantic consistency and traceability.
- **Transform SysML to AUTOSAR** — Rhapsody supports model-to-model transformation from SysML system architecture to AUTOSAR software architecture.
- **Import/export XMI** (v1) — exchange models between tools using the standard XMI interchange format (though practical interoperability varies).
- **Import/export SysML v2 textual notation** — parse `.sysml` and `.kerml` text files, enabling text-based authoring, version control in Git, and tool-to-tool exchange via a human-readable format.

---

## 3. Views and diagrams: what users can see and navigate

### 3.1 Standard SysML v1 diagram types (9 diagrams)

**Block Definition Diagram (BDD).** Shows the taxonomy and composition of system structure. Users can visualize block hierarchies, generalizations, associations, compositions, value types, constraint blocks, flow specifications, and interface blocks. Compartments show properties, operations, ports, and constraints.

**Internal Block Diagram (IBD).** Shows the internal structure of a single block — its parts, ports, connectors, and item flows. Users can visualize how components are wired together, what flows between them, and the delegation from outer ports to inner parts.

**Activity Diagram.** Shows behavioral flow — actions, control flows, object flows, decision/merge nodes, fork/join nodes, pins, activity partitions (swimlanes), and call behaviors. Users can visualize functional decomposition and data/control flow through a process.

**State Machine Diagram.** Shows the lifecycle states of a block — states, transitions, triggers, guards, effects, composite states, orthogonal regions, and pseudostates. Users can visualize how a component responds to events over time.

**Sequence Diagram.** Shows time-ordered message exchanges between parts — lifelines, messages (synchronous, asynchronous, reply), combined fragments (alt, loop, opt, par, break, etc.), and interaction references. Users can visualize specific scenarios of system operation.

**Use Case Diagram.** Shows system capabilities from the stakeholder perspective — actors, use cases, system boundary, include/extend relationships. Users can visualize what the system does for whom.

**Requirement Diagram.** Shows requirements and their relationships — containment/decomposition, derive, satisfy, verify, refine, trace, and copy. Users can visualize the requirements architecture and its traceability to design and test.

**Parametric Diagram.** Shows a network of constraint blocks bound to part properties — parameters, binding connectors, and constraint expressions. Users can visualize the analysis setup linking equations to system values.

**Package Diagram.** Shows model organization — packages, dependencies, imports, containment. Users can visualize the model's top-level structure and inter-package dependencies.

### 3.2 SysML v2 view and viewpoint mechanism

SysML v2 makes **views and viewpoints first-class model elements** rather than just diagram artifacts:

- **Define viewpoint definitions** specifying what a particular stakeholder needs to see — including filter criteria, rendering rules, and the concerns addressed. A viewpoint is a reusable specification: "show all parts with their mass and power properties, linked to their requirements."
- **Define view definitions and usages** that *render* model content according to a viewpoint. A view is the concrete instantiation — it selects elements from the model based on the viewpoint's filter and presents them.
- **Use expose and filter mechanisms** to control exactly which elements appear in a view — by type, by relationship, by package scope, by metadata annotation, or by custom query expression.
- **Map stakeholder concerns to viewpoints** — formalizing which stakeholders need which information and generating views that satisfy those concerns.
- **Render views as either graphical diagrams or textual output** — the same view definition can produce a diagram, a table, or a text document depending on the rendering specification.
- **Maintain consistency between views and model** — since views are derived from the underlying semantic model, changes to the model automatically propagate to all views.

### 3.3 Tabular views

- **Requirements tables** — display requirements in a spreadsheet-like grid with columns for ID, text, priority, risk, status, satisfying elements, and verifying elements. Sortable, filterable, and editable.
- **Property/specification tables** — show all properties of selected blocks/parts in a tabular format, enabling rapid comparison and bulk editing of value properties, types, and defaults.
- **Allocation tables** — show which functions are allocated to which components in a two-column or matrix layout.
- **Custom element tables** — construct tables from arbitrary model queries, displaying any combination of element types and attributes. Cameo, Sparx EA, and Rhapsody all support generic table views from model content.
- **Excel/CSV import and export** — synchronize tabular model data with spreadsheets for stakeholder review or data-entry workflows. Cameo's Excel/CSV Sync enables bidirectional sync including connector import.

### 3.4 Matrix views

- **Traceability matrices** — cross-reference requirements against design elements (satisfy), test cases (verify), or other requirements (derive). Rows = source, columns = target, cells = relationship existence. Available as predefined matrices in Cameo, Sparx EA, and Rhapsody.
- **N² (N-squared) diagrams / dependency structure matrices** — show interface dependencies between components, with components on both axes and interfaces/flows in the cells. Critical for interface management.
- **Allocation matrices** — map functions to components, software to hardware, or logical to physical in a matrix format.
- **Relationship matrices** — generalized matrix showing any cross-relationship between any two sets of elements. Users configure row/column element types and the relationship of interest. Sparx EA's Relationship Matrix is particularly flexible.
- **Impact analysis matrices** — show which elements are affected when a specific element changes, traversing dependency and traceability relationships.

### 3.5 Dashboard and summary views

- **Model metrics dashboards** — display element counts by type, diagram counts, model size statistics, and growth trends over time.
- **Requirements coverage dashboards** — show percentage of requirements with satisfy links, verify links, and derived/refined relationships. Highlight orphan requirements and untested requirements.
- **Verification status dashboards** — show pass/fail/pending status of verification cases against requirements, with drill-down to individual results.
- **Model health/quality dashboards** — display validation error counts, completeness metrics, naming convention compliance, and conformance to modeling guidelines.
- **Custom widget dashboards** — Cameo Collaborator and Sparx Prolaborate enable configurable dashboards with charts, tables, and diagram thumbnails for stakeholder-specific views.

### 3.6 Model browsers and explorers

- **Containment tree browser** — the primary navigation tree showing the model's package/element hierarchy. Every tool provides this as the central navigation mechanism.
- **Relationship/dependency browsers** — explore all incoming and outgoing relationships of a selected element — satisfies, verifies, derives, allocates, traces, specializes, etc.
- **Impact analysis views** — starting from a selected element, show all directly and transitively dependent elements that would be affected by a change.
- **Search and query result views** — execute text searches, OCL queries, or structured queries and display matching elements in a filterable list.
- **Cross-reference views** — for a given element, show everywhere it appears — in which diagrams, which packages, which relationships.
- **Model diff/comparison views** — compare two model versions side-by-side, highlighting added, removed, and modified elements with merge capabilities. Cameo provides graphical diff; Sparx EA provides model baselining and comparison.
- **Diagram navigation maps** — overview maps showing all diagrams and their interconnections for wayfinding in large models.
- **Advanced model browsers** (Rhapsody) — sortable, filterable, and editable model element browsers with customizable column layouts.

### 3.7 Non-standard but widely supported visualizations

- **Context diagrams** — show a system block with its external actors, interfaces, and interactions in a simplified view. Not a standard SysML diagram type but universally created using BDD or IBD.
- **Hierarchical decomposition trees** — display part decomposition as a tree or indented list, visualizing the WBS-like breakdown of a system.
- **Interconnect/wiring diagrams** — tool-specific detailed views showing physical connections between components, particularly for electrical/electronic systems.
- **Timeline/schedule views** — visualize temporal aspects of behavior, showing when actions execute and states are active along a time axis.
- **Mind map views** — Sparx EA supports mind map diagrams for brainstorming and early requirements elicitation.
- **Kanban/task boards** — Sparx EA supports linking SysML v2 elements to Kanban tasks, user stories, and Gantt chart activities via the SysML v2 reference object.
- **PlantUML-rendered diagrams** — the SysML v2 pilot implementation uses PlantUML to generate preliminary graphical views of textual SysML v2 models in both Eclipse and Jupyter environments.
- **Tom Sawyer SysML v2 Viewer** — a dedicated web-based visualization tool that renders any model published to the SysML v2 API repository with interactive navigation.

### 3.8 Diagram customization and export

- **Symbol/notation customization** — change element colors, shapes, font sizes, compartment visibility, line styles, and icon overlays.
- **Conditional formatting** — color-code elements based on property values, stereotype, status, or custom criteria (e.g., red = non-compliant, green = verified).
- **Diagram templates** — create reusable diagram templates with predefined layouts, legends, and element palettes for organizational consistency.
- **Auto-layout** — automatically arrange diagram elements using force-directed, hierarchical, or orthogonal layout algorithms.
- **Layer management** — show/hide diagram layers to control information density per audience.
- **Hyperlinks between diagrams** — navigate from an element on one diagram to a related diagram showing that element's details.
- **Export to image formats** — PNG, SVG, PDF, EMF for embedding in documents and presentations.
- **Interactive web publication** — Cameo Collaborator for Teamwork Cloud and Sparx Prolaborate publish interactive model views to browsers for stakeholder review without tool licenses.
- **Presentation generation** — export selected diagrams and tables as PowerPoint slides or HTML pages for design reviews.

---

## 4. Major tools and their unique capabilities

### Cameo Systems Modeler / CATIA Magic (Dassault Systèmes)

The market-leading SysML tool. **Cameo now supports both SysML v1 and v2** (2026x release) with claimed 100% standard conformance to the v2 metamodel and true two-way synchronization between textual and graphical notation. Key differentiators: **Cameo Simulation Toolkit** (fUML execution, state machine animation, parametric solving, Monte Carlo analysis, trade studies, rollup calculations); **Teamwork Cloud** for multi-user collaborative modeling with branching, versioning, and access control; **Cameo DataHub** for integration with DOORS, RequisitePro, and CSV sources; **Cameo Collaborator** for web-based stakeholder review; **Report Wizard/DocGen** for document generation; scripting in Jython, Groovy, JavaScript, BeanShell; **DSL Customization Engine** for domain-specific profiles; SSP file export for Dymola co-simulation; **ISO 26262 TCL2 software certification**; ReqIF import/export; and the **3DEXPERIENCE platform integration** for enterprise PLM connectivity.

### IBM Rhapsody and Rhapsody Systems Engineering

Two products now coexist. **Classic Rhapsody (10.0.1)** remains the desktop-based tool supporting SysML v1, UML, AUTOSAR, with the unique capability of **production code generation in C, C++, Java, and Ada** — TÜV-certified for safety standards. It provides model execution/animation, round-trip engineering, parametric constraint solving, and deep integration with IBM Engineering Lifecycle Management (DOORS Next, Workflow Management, Test Management) via OSLC. **Rhapsody SE** is the new cloud-native, web-based platform built for SysML v2, featuring GraphQL and RESTful APIs, containerized deployment, HarmonyMBE methodology integration, intuitive drag-and-drop with auto-model-completion, JavaScript/Python extensibility, global configuration management, and SysML v1 model reuse/migration to v2. **FMI/FMU co-simulation** is supported. Rhapsody SE v1.5 (October 2025) emphasized simplifying v2 adoption with intelligent automation of port, parameter, and element creation.

### Sparx Systems Enterprise Architect + Trechoro

The most affordable commercial option (perpetual licensing, ~$229–$899 per seat depending on edition). **Enterprise Architect** supports SysML v1.5, UML 2.5, BPMN, ArchiMate 3.2, and many other standards in a single repository. SysML v2 support is delivered through **Trechoro**, a dedicated modeling environment introduced at the 2025 Global Summit, built natively on KerML (no UML layer). Trechoro supports textual notation import/export, graphical diagramming, SysML v2 reference objects for cross-referencing v1/v2 and Kanban/Gantt elements, syntax highlighting in a code editor, and COM/.NET APIs for automation. The v1 platform provides state machine execution, parametric simulation, MDA transformations, built-in requirements management, Relationship Matrix, rich document generation, model patterns, and **Prolaborate** for web-based stakeholder dashboards. Sparx Pro Cloud Server enables distributed team performance optimization. The roadmap includes API and automation extensions, simulation, metadata-driven dynamic diagrams, and advanced editing.

### Eclipse Papyrus

**Open-source** (Eclipse Public License) SysML v1.6 and UML 2.5 modeling environment within the Eclipse ecosystem. Highly extensible via Java plugins and Eclipse EMF. Version 7.0.0 (June 2025) introduced enhanced UML diagrams via Sirius integration. Supports all standard SysML v1 diagram types, profile definition and application, and integration with Capella for the Arcadia methodology. Primary limitations compared to commercial tools: no built-in simulation, weaker document generation, less polished UI, and smaller support community. Strength is in academic use, research projects, and organizations preferring open-source infrastructure.

### SysML v2 Pilot Implementation (OMG/SST)

The **reference implementation** maintained by the OMG Systems Modeling Community's Reference Implementation Working Group. Available in two forms: **Eclipse-based editor** (Xtext-based textual editor with syntax highlighting, validation, and PlantUML visualization for SysML v2 graphical notation) and **Jupyter Notebook kernel** (interactive cells for writing SysML v2 textual notation, executing `%show` and `%viz` commands, and publishing models to a repository). The **SysML v2 API Services** pilot implementation provides a REST/HTTP API (Swagger-documented, PostgreSQL-backed) for CRUD operations on models, project/commit management, and element querying. Accessible at sysmlv2lab.com for online experimentation. Licensed under LGPL v3.0. Not a production tool — it is a proof-of-concept for specification validation and community learning.

### Other notable tools and platforms

- **PTC Modeler** (10.2, Summer 2025) — commercial SysML v2 tool from PTC for safety-critical systems design, verification, and maintenance.
- **Ansys System Architecture Modeler** — SysML v2-based tool integrated into the Ansys simulation ecosystem, with safety/cybersecurity analysis capabilities including model-based FMEA and FTA.
- **Intercax Syndeia** (3.6) — not a SysML authoring tool but the leading **digital thread platform** connecting SysML models (v1 and v2) with PLM, ALM, CAD, simulation, requirements, and test tools. First commercial tool to support SysML v2 API (2021). Provides model transformation, bidirectional synchronization, cross-tool traceability visualization, and federated digital thread queries.
- **Tom Sawyer SysML v2 Viewer** — web-based visualization for SysML v2 models published to a repository via the standard API.
- **SysIDE** — VS Code extension for SysML v2 textual notation authoring, usable standalone or integrated into other tools.
- **Celedon Davinci** — emerging AI-powered engineering platform using SysML v2 as its foundation for automated system design from concept through manufacturing.
- **Innoslate** — web-based MBSE tool with its own metamodel (not strict SysML but SysML-compatible), focused on requirements, behavior, and architecture modeling with document import capabilities.
- **Vitech GENESYS** — entity-relationship-based MBSE tool (not native SysML) with system architecture, behavior, requirements, and V&V capabilities.
- **Capella/Arcadia** (Eclipse) — not SysML-based but a prominent open-source MBSE approach using the Arcadia methodology, often compared to and used alongside SysML tools.
- **Sodius/Willert** — provides model transformation bridges between IBM Rhapsody, Cameo, Sparx EA, and UNICOM System Architect, including forthcoming SysML v2 support.

---

## What this means for a v2 tool implementation

The catalogue above represents the **full capability envelope** that users expect from a modern SysML v2 tool. A few patterns emerge from the landscape. First, **textual-graphical synchronization** is now table stakes — Cameo, Rhapsody SE, Trechoro, and the pilot implementation all provide it. Second, the **SysML v2 API** is the critical interoperability mechanism; any new tool must implement it to participate in the digital engineering ecosystem. Third, **simulation and execution capabilities** represent the highest-value differentiator — Cameo's Simulation Toolkit and Rhapsody's code generation/execution are what make models actionable rather than just descriptive. Fourth, **views are no longer just diagrams** — tables, matrices, dashboards, and web-published stakeholder views are equally essential. Finally, the market is segmenting: heavyweight commercial tools (Cameo, Rhapsody SE) serve large enterprises; Sparx EA serves cost-sensitive teams; the pilot implementation and SysIDE serve experimenters and developers; and Syndeia serves integration-first workflows. A new v2 tool must decide where in this landscape it competes and which capabilities from this catalogue to prioritize.