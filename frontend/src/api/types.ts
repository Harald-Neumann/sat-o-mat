export type TaskState = 'Active' | 'PendingApproval' | 'Completed' | 'Failed';

export interface TaskListEntry {
  id: string;
  state: TaskState;
  start: string | null;
  end: string | null;
}

export interface ApiPass {
  start: string;
  end: string;
  azimuth: number[];
  elevation: number[];
}

export interface PassPredictions {
  predictions: Record<string, ApiPass[]>;
}

export interface ApiGroundTrack {
  start: string;
  latitude: number[];
  longitude: number[];
}

export interface GroundTrackPredictions {
  predictions: Record<string, ApiGroundTrack>;
}
