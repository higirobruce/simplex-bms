"use client";

import * as React from "react";
import { ChevronRight, Plus, Pencil, Trash2, Warehouse } from "lucide-react";
import { cn } from "@/lib/utils";

export interface WarehouseNode {
  id: string;
  name: string;
  code?: string | null;
  parentId: string | null;
  directQty: number;
}

export interface TreeNode extends WarehouseNode {
  children: TreeNode[];
  totalQty: number;
}

export function buildTree(nodes: WarehouseNode[]): TreeNode[] {
  const map = new Map<string, TreeNode>();
  nodes.forEach((n) =>
    map.set(n.id, { ...n, children: [], totalQty: n.directQty })
  );
  const roots: TreeNode[] = [];
  map.forEach((node) => {
    if (node.parentId && map.has(node.parentId)) {
      map.get(node.parentId)!.children.push(node);
    } else {
      roots.push(node);
    }
  });
  // Compute totalQty bottom-up
  const computeTotals = (node: TreeNode): number => {
    const childSum = node.children.reduce((s, c) => s + computeTotals(c), 0);
    node.totalQty = node.directQty + childSum;
    return node.totalQty;
  };
  roots.forEach(computeTotals);
  // Sort siblings alphabetically
  const sortNode = (n: TreeNode) => {
    n.children.sort((a, b) => a.name.localeCompare(b.name));
    n.children.forEach(sortNode);
  };
  roots.sort((a, b) => a.name.localeCompare(b.name));
  roots.forEach(sortNode);
  return roots;
}

interface WarehouseTreeProps {
  nodes: TreeNode[];
  selectedId?: string | null;
  onSelect?: (id: string) => void;
  onAddChild?: (parentId: string) => void;
  onEdit?: (node: TreeNode) => void;
  onDelete?: (node: TreeNode) => void;
}

export function WarehouseTree({
  nodes,
  selectedId,
  onSelect,
  onAddChild,
  onEdit,
  onDelete,
}: WarehouseTreeProps) {
  return (
    <ul className="space-y-0.5">
      {nodes.map((node) => (
        <TreeNodeItem
          key={node.id}
          node={node}
          depth={0}
          selectedId={selectedId}
          onSelect={onSelect}
          onAddChild={onAddChild}
          onEdit={onEdit}
          onDelete={onDelete}
        />
      ))}
    </ul>
  );
}

function TreeNodeItem({
  node,
  depth,
  selectedId,
  onSelect,
  onAddChild,
  onEdit,
  onDelete,
}: {
  node: TreeNode;
  depth: number;
  selectedId?: string | null;
  onSelect?: (id: string) => void;
  onAddChild?: (parentId: string) => void;
  onEdit?: (node: TreeNode) => void;
  onDelete?: (node: TreeNode) => void;
}) {
  const [open, setOpen] = React.useState(true);
  const hasChildren = node.children.length > 0;
  const isActive = selectedId === node.id;

  return (
    <li>
      <div
        className={cn(
          "group flex items-center gap-2 rounded-xl pr-2 transition-colors",
          isActive
            ? "bg-accent-soft"
            : "hover:bg-surface-2"
        )}
        style={{ paddingLeft: `${depth * 14 + 6}px` }}
      >
        <button
          onClick={() => hasChildren && setOpen(!open)}
          className={cn(
            "flex h-6 w-6 items-center justify-center rounded-md text-ink-mute transition-transform",
            !hasChildren && "invisible"
          )}
          aria-label={open ? "Collapse" : "Expand"}
        >
          <ChevronRight
            className={cn("h-3.5 w-3.5 transition-transform", open && "rotate-90")}
            strokeWidth={1.75}
          />
        </button>
        <button
          onClick={() => onSelect?.(node.id)}
          className="flex flex-1 items-center gap-2 py-1.5 text-left text-sm"
        >
          <Warehouse
            className={cn(
              "h-3.5 w-3.5 shrink-0",
              isActive ? "text-accent" : "text-ink-mute"
            )}
            strokeWidth={1.75}
          />
          <span
            className={cn(
              "truncate",
              isActive ? "text-accent font-medium" : "text-ink"
            )}
          >
            {node.name}
          </span>
          {node.code && (
            <span className="text-[0.65rem] tracking-[0.12em] uppercase text-ink-mute">
              {node.code}
            </span>
          )}
          <span className="ml-auto tabular text-xs text-ink-mute">
            {node.totalQty}
          </span>
        </button>
        <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
          {onAddChild && (
            <button
              onClick={() => onAddChild(node.id)}
              className="rounded-md p-1 text-ink-mute hover:bg-surface-3 hover:text-ink"
              title="Add sub-location"
              aria-label="Add sub-location"
            >
              <Plus className="h-3 w-3" strokeWidth={1.75} />
            </button>
          )}
          {onEdit && (
            <button
              onClick={() => onEdit(node)}
              className="rounded-md p-1 text-ink-mute hover:bg-surface-3 hover:text-ink"
              title="Edit"
              aria-label="Edit"
            >
              <Pencil className="h-3 w-3" strokeWidth={1.75} />
            </button>
          )}
          {onDelete && (
            <button
              onClick={() => onDelete(node)}
              className="rounded-md p-1 text-ink-mute hover:bg-surface-3 hover:text-danger"
              title="Delete"
              aria-label="Delete"
            >
              <Trash2 className="h-3 w-3" strokeWidth={1.75} />
            </button>
          )}
        </div>
      </div>
      {hasChildren && open && (
        <ul className="space-y-0.5">
          {node.children.map((child) => (
            <TreeNodeItem
              key={child.id}
              node={child}
              depth={depth + 1}
              selectedId={selectedId}
              onSelect={onSelect}
              onAddChild={onAddChild}
              onEdit={onEdit}
              onDelete={onDelete}
            />
          ))}
        </ul>
      )}
    </li>
  );
}
