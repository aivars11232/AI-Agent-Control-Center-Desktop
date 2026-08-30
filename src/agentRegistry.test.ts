import { describe, expect, it } from "vitest";
import {
  availableAgentGroups,
  findActiveTemplateAgent,
  normalizeLegacyAgentRegistrySet,
  projectAgentGroup,
  registryIssueMessage,
  validManagerCandidates,
} from "./agentRegistry";
import { agentFixture } from "./test/fixtures";

describe("TASK-0009 agent registry projections", () => {
  it("keeps custom agents visible and derives ancestor context without fixed ids", () => {
    const supervisor = agentFixture({
      id: 101,
      name: "Custom Supervisor",
      role: "Supervisor",
      category: "Management",
      reportsTo: null,
      authorityLevel: 4,
    });
    const senior = agentFixture({
      id: 202,
      name: "Custom Senior",
      role: "Senior Agent",
      reportsTo: supervisor.id,
      authorityLevel: 2,
    });
    const specialist = agentFixture({
      id: 303,
      name: "Custom Specialist",
      reportsTo: senior.id,
    });

    const projection = projectAgentGroup(
      [supervisor, senior, specialist],
      "Development",
    );

    expect(projection.memberIds).toEqual(new Set([senior.id, specialist.id]));
    expect(projection.visibleAgents.map((agent) => agent.id)).toEqual([
      supervisor.id,
      senior.id,
      specialist.id,
    ]);
    expect(projection.rows.map(({ agent, depth }) => [agent.id, depth])).toEqual([
      [supervisor.id, 0],
      [senior.id, 1],
      [specialist.id, 2],
    ]);
  });

  it("separates active, unassigned, and deleted identities", () => {
    const active = agentFixture({
      id: 12,
      name: "Active",
      role: "Supervisor",
      category: "Management",
      authorityLevel: 4,
      reportsTo: null,
    });
    const unassigned = agentFixture({
      id: 13,
      name: "Needs Repair",
      registryState: "unassigned",
      registryIssue: "missing-manager",
      status: "Paused",
      reportsTo: null,
    });
    const deleted = agentFixture({
      id: 14,
      name: "Deleted",
      registryState: "deleted",
      status: "Paused",
      reportsTo: null,
      deletedAtUnixMs: 1,
    });
    const agents = normalizeLegacyAgentRegistrySet([active, unassigned, deleted]);

    expect(availableAgentGroups(agents)).toContain("Needs assignment");
    expect(projectAgentGroup(agents, "All agents").visibleAgents).toEqual([
      active,
    ]);
    expect(projectAgentGroup(agents, "Needs assignment").visibleAgents).toEqual([
      unassigned,
    ]);
    expect(agents.find((agent) => agent.id === deleted.id)).toEqual(deleted);
  });

  it("uses stable template identity after a display-name change", () => {
    const renamed = agentFixture({
      id: 77,
      name: "Desktop Operator",
      templateKey: "pc-control",
    });
    const deleted = agentFixture({
      id: 78,
      templateKey: "coding",
      registryState: "deleted",
      status: "Paused",
      deletedAtUnixMs: 1,
    });

    expect(findActiveTemplateAgent([renamed, deleted], "pc-control")).toBe(renamed);
    expect(findActiveTemplateAgent([renamed, deleted], "coding")).toBeNull();
  });

  it("offers only active, higher-authority, non-descendant managers", () => {
    const supervisor = agentFixture({
      id: 1,
      role: "Supervisor",
      authorityLevel: 4,
    });
    const leader = agentFixture({
      id: 2,
      role: "Team Leader",
      authorityLevel: 3,
      reportsTo: supervisor.id,
    });
    const senior = agentFixture({
      id: 3,
      role: "Senior Agent",
      authorityLevel: 2,
      reportsTo: leader.id,
    });
    const deletedLeader = agentFixture({
      id: 4,
      role: "Team Leader",
      authorityLevel: 3,
      registryState: "deleted",
      status: "Paused",
      reportsTo: null,
      deletedAtUnixMs: 1,
    });

    expect(
      validManagerCandidates(
        [supervisor, leader, senior, deletedLeader],
        "Specialist",
        null,
      ).map((agent) => agent.id),
    ).toEqual([supervisor.id, leader.id, senior.id]);
    expect(
      validManagerCandidates(
        [supervisor, leader, senior, deletedLeader],
        "Specialist",
        leader.id,
      ).map((agent) => agent.id),
    ).toEqual([supervisor.id]);
  });

  it("quarantines legacy cycles and dangling managers without hiding agents", () => {
    const supervisor = agentFixture({
      id: 40,
      role: "Supervisor",
      authorityLevel: 4,
      reportsTo: null,
    });
    const senior = agentFixture({
      id: 41,
      role: "Senior Agent",
      authorityLevel: 2,
      reportsTo: 42,
    });
    const specialist = agentFixture({ id: 42, reportsTo: senior.id });
    const dangling = agentFixture({ id: 43, reportsTo: 999_999 });

    const migrated = normalizeLegacyAgentRegistrySet([
      supervisor,
      senior,
      specialist,
      dangling,
    ]);

    expect(
      migrated
        .filter((agent) => agent.registryIssue === "cycle")
        .map((agent) => agent.id),
    ).toEqual([senior.id, specialist.id]);
    expect(migrated.find((agent) => agent.id === dangling.id)).toMatchObject({
      registryState: "unassigned",
      registryIssue: "missing-manager",
      reportsTo: null,
    });
    expect(projectAgentGroup(migrated, "Needs assignment").visibleAgents).toHaveLength(3);
  });
});

describe("TASK-0021 duplicate identity recovery", () => {
  it("recognizes the duplicate-id registry issue with its own message", () => {
    expect(registryIssueMessage("duplicate-id")).toBe(
      "This agent shared an identifier with another agent during migration and was given a new identity for review.",
    );
  });

  it("keeps a re-keyed duplicate quarantined without relabeling the issue", () => {
    const supervisor = agentFixture({
      id: 1,
      role: "Supervisor",
      category: "Management",
      authorityLevel: 4,
      reportsTo: null,
    });
    const canonical = agentFixture({
      id: 5,
      name: "Finance Agent",
      category: "Finance",
      reportsTo: 1,
    });
    const requarantined = agentFixture({
      id: 12,
      name: "Financial Agent",
      category: "Finance",
      registryState: "unassigned",
      registryIssue: "duplicate-id",
      status: "Paused",
      reportsTo: null,
    });

    const migrated = normalizeLegacyAgentRegistrySet([
      supervisor,
      canonical,
      requarantined,
    ]);

    expect(migrated.find((agent) => agent.id === 12)).toMatchObject({
      registryState: "unassigned",
      registryIssue: "duplicate-id",
      reportsTo: null,
    });
    expect(migrated.find((agent) => agent.id === 5)).toMatchObject({
      registryState: "active",
      registryIssue: null,
    });
  });
});
