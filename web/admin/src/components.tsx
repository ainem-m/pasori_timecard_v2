import { ChevronRight, type LucideIcon } from 'lucide-react';

import { cn } from './utils';

export const SidebarItem = ({
  icon: Icon,
  label,
  active,
  onClick,
}: {
  icon: LucideIcon;
  label: string;
  active?: boolean;
  onClick: () => void;
}) => (
  <button
    onClick={onClick}
    className={cn(
      'flex items-center gap-3 w-full px-4 py-3 rounded-lg transition-all duration-200 group',
      active
        ? 'bg-primary/10 text-primary font-medium'
        : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
    )}
  >
    <Icon className={cn('w-5 stealth-5 transition-transform group-hover:scale-110', active && 'text-primary')} />
    <span>{label}</span>
    {active && <ChevronRight className="ml-auto w-4 h-4" />}
  </button>
);

export const StatCard = ({
  title,
  value,
  icon: Icon,
  trend,
}: {
  title: string;
  value: string | number;
  icon: LucideIcon;
  trend?: string;
}) => (
  <div className="card p-6 flex items-start justify-between">
    <div>
      <p className="text-sm font-medium text-muted-foreground mb-1">{title}</p>
      <h3 className="text-3xl font-bold tracking-tight">{value}</h3>
      {trend && <p className="text-xs text-green-500 mt-2 font-medium">{trend}</p>}
    </div>
    <div className="p-3 bg-accent rounded-lg">
      <Icon className="w-6 h-6 text-accent-foreground" />
    </div>
  </div>
);
